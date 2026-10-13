// Document creation and immutable revision append (plan/03 CONTENT-01).
use super::*;

impl Store {
    pub async fn create_document(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: DocumentCreate,
        request_hash: &str,
    ) -> Result<Replay<Document>, DomainError> {
        let actor_id = ctx.actor.id;
        let now = ctx.now;
        let command_id = ctx.command_id;

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let owner = resolve_owner_scope(tx, &project, input.owner).await?;
                let actor = live_actor(tx, &actor_id).await?;
                require_capability(&actor, Capability::AuthorDocuments)?;
                owner.ensure_unarchived()?;
                owner.ensure_kind_allowed(input.kind)?;
                let title = validate_document_create(&input)?;

                let document_id = DocumentId::generate(now);
                let revision_id = DocumentRevisionId::generate(now);
                let links_text = links_json(&input.links)?;

                // Circular FK (documents.latest_revision_id <-> revisions.document_id)
                // is DEFERRABLE INITIALLY DEFERRED; both rows commit together.
                sqlx::query(
                    "INSERT INTO documents (id, revision, created_at, updated_at, project_id, \
                     owner_kind, owner_id, kind, title, latest_revision_id) \
                     VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .bind(document_id.to_string())
                .bind(&now_text)
                .bind(project.to_string())
                .bind(input.owner.kind().as_str())
                .bind(input.owner.id().to_string())
                .bind(input.kind.as_str())
                .bind(&title)
                .bind(revision_id.to_string())
                .execute(&mut **tx)
                .await?;

                sqlx::query(
                    "INSERT INTO document_revisions (id, document_id, number, body, links, \
                     actor_id, created_at) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)",
                )
                .bind(revision_id.to_string())
                .bind(document_id.to_string())
                .bind(&input.body)
                .bind(&links_text)
                .bind(actor_id.to_string())
                .bind(&now_text)
                .execute(&mut **tx)
                .await?;

                let events_ctx = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                append_events(
                    tx,
                    &project,
                    &events_ctx,
                    "createDocument",
                    "",
                    vec![PendingEvent::document(
                        document_id,
                        Revision::INITIAL,
                        input.owner.id(),
                    )],
                )
                .await?;

                document_row(tx, &project, &document_id)
                    .await?
                    .ok_or_else(|| missing_after_write("document"))
            })
        })
        .await
    }

    pub async fn create_document_revision(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        document_id: DocumentId,
        input: RevisionCreate,
        request_hash: &str,
    ) -> Result<Replay<DocumentRevision>, DomainError> {
        let actor_id = ctx.actor.id;
        let now = ctx.now;
        let command_id = ctx.command_id;
        let expected_revision = ctx.expected_revision;

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let document = document_row(tx, &project, &document_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let actor = live_actor(tx, &actor_id).await?;
                require_capability(&actor, Capability::AuthorDocuments)?;
                require_revision(expected_revision, document.revision)?;
                let owner = resolve_owner_scope(
                    tx,
                    &project,
                    DocumentOwner::from_parts(document.owner_kind, document.owner_id),
                )
                .await?;
                owner.ensure_unarchived()?;
                owner.ensure_kind_allowed(document.kind)?;
                if let Some(task_id) = owner.task_id
                    && is_selected_plan_document(tx, task_id, &document_id).await?
                    && task_execute_or_review_work(tx, task_id, &now_text).await?
                {
                    return Err(DomainError::ActiveWork(
                        "the selected plan cannot gain revisions during execute/review \
                         claims or a pending submission"
                            .into(),
                    ));
                }
                validate_revision_create(&input)?;

                let revision_id = DocumentRevisionId::generate(now);
                // Invariant: documents.revision always equals the latest revision number.
                let number = document.revision.value() + 1;
                let links_text = links_json(&input.links)?;

                sqlx::query(
                    "INSERT INTO document_revisions (id, document_id, number, body, links, \
                     actor_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .bind(revision_id.to_string())
                .bind(document_id.to_string())
                .bind(number)
                .bind(&input.body)
                .bind(&links_text)
                .bind(actor_id.to_string())
                .bind(&now_text)
                .execute(&mut **tx)
                .await?;

                sqlx::query(
                    "UPDATE documents SET revision = revision + 1, updated_at = ?1, \
                     latest_revision_id = ?2 WHERE id = ?3",
                )
                .bind(&now_text)
                .bind(revision_id.to_string())
                .bind(document_id.to_string())
                .execute(&mut **tx)
                .await?;

                let events_ctx = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                append_events(
                    tx,
                    &project,
                    &events_ctx,
                    "createDocumentRevision",
                    "",
                    vec![PendingEvent::document(
                        document_id,
                        document.revision.next(),
                        document.owner_id,
                    )],
                )
                .await?;

                revision_row(tx, &project, &revision_id)
                    .await?
                    .ok_or_else(|| missing_after_write("document revision"))
            })
        })
        .await
    }
}
