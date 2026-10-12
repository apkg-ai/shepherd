use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Capability, Document, DocumentCreate, DocumentId, DocumentKind, DocumentOwner,
    DocumentOwnerKind, DocumentRevision, DocumentRevisionId, ProjectId, Revision, RevisionCreate,
    SelectPlanInput, Task, TaskId, TaskPhase, require_capability, validate_document_create,
    validate_revision_create,
};
use crate::queries::hierarchy::{project_row, task_row};
use crate::queries::{ListParams, Page, decode_cursor, effective_limit, encode_cursor, split_page};
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

use super::{
    CommandContext, CommandResult, PendingEvent, Replay, append_events, epic_scope, live_actor,
    missing_after_write, require_revision, task_scope,
};

/// Validate that every UUID in `revision_ids` belongs to a document owned by the task.
pub(super) async fn validate_task_owned_revisions(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
    revision_ids: &[Uuid],
) -> Result<(), DomainError> {
    if revision_ids.is_empty() {
        return Ok(());
    }
    let task_str = task.to_string();
    let project_str = project.to_string();
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT dr.id FROM document_revisions dr \
         JOIN documents d ON d.id = dr.document_id \
         WHERE d.owner_kind = 'task' AND d.owner_id = ",
    );
    builder.push_bind(&task_str);
    builder.push(" AND d.project_id = ");
    builder.push_bind(&project_str);
    builder.push(" AND dr.id IN (");
    let mut separated = builder.separated(", ");
    for id in revision_ids {
        separated.push_bind(id.to_string());
    }
    builder.push(")");
    let found: Vec<(String,)> = builder
        .build_query_as()
        .fetch_all(&mut *conn)
        .await
        .map_err(StorageError::from)?;
    if found.len() != revision_ids.len() {
        let found_set: std::collections::HashSet<&str> =
            found.iter().map(|r| r.0.as_str()).collect();
        let missing = revision_ids
            .iter()
            .find(|id| !found_set.contains(id.to_string().as_str()))
            .map(Uuid::to_string)
            .unwrap_or_else(|| "unknown".into());
        return Err(DomainError::Validation {
            field: "document_revision_ids",
            message: format!("revision {missing} does not belong to a document owned by this task"),
        });
    }
    Ok(())
}

/// The selection target must be a revision of a plan document on that task (plan/03).
pub(super) async fn require_task_plan_revision(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
    revision_id: &Uuid,
) -> Result<(), DomainError> {
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT d.kind FROM documents d \
         JOIN document_revisions dr ON dr.document_id = d.id \
         WHERE dr.id = ?1 AND d.project_id = ?2 \
         AND d.owner_kind = 'task' AND d.owner_id = ?3",
    )
    .bind(revision_id.to_string())
    .bind(project.to_string())
    .bind(task.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    match kind.as_deref() {
        None => Err(DomainError::Validation {
            field: "document_revision_id",
            message: "does not belong to a document owned by this task".into(),
        }),
        Some("plan") => Ok(()),
        Some(_) => Err(DomainError::Validation {
            field: "document_revision_id",
            message: "must reference a plan document".into(),
        }),
    }
}

// Narrower than task_has_active_work: plan claims block neither selection nor
// selected-plan appends (plan/03, plan/04 row "select plan").
pub(super) async fn task_execute_or_review_work(
    conn: &mut SqliteConnection,
    task: TaskId,
    now: &str,
) -> Result<bool, DomainError> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM tasks t WHERE t.id = ?1 AND \
         (EXISTS (SELECT 1 FROM claims c WHERE c.task_id = t.id AND c.status = 'active' \
         AND c.expires_at > ?2 AND c.phase IN ('execute', 'review')) \
         OR EXISTS (SELECT 1 FROM submissions s WHERE s.task_id = t.id \
         AND s.status = 'pending')) LIMIT 1",
    )
    .bind(task.to_string())
    .bind(now)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

async fn is_selected_plan_document(
    conn: &mut SqliteConnection,
    task: TaskId,
    document: &DocumentId,
) -> Result<bool, DomainError> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM tasks t \
         JOIN document_revisions dr ON dr.id = t.selected_plan_revision_id \
         WHERE t.id = ?1 AND dr.document_id = ?2 LIMIT 1",
    )
    .bind(task.to_string())
    .bind(document.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

struct DocumentOwnerScope {
    archived: bool,
    terminal: bool,
    task_id: Option<TaskId>,
}

impl DocumentOwnerScope {
    fn ensure_unarchived(&self) -> Result<(), DomainError> {
        if self.archived {
            return Err(DomainError::ArchivedScope);
        }
        Ok(())
    }

    fn ensure_kind_allowed(&self, kind: DocumentKind) -> Result<(), DomainError> {
        if self.terminal && !kind.allowed_on_terminal_owner() {
            return Err(DomainError::TerminalScope);
        }
        Ok(())
    }
}

// Membership only (404); archived/terminal are flags so commands keep the
// plan/04 precedence (capability and revision first).
async fn resolve_owner_scope(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    owner: DocumentOwner,
) -> Result<DocumentOwnerScope, DomainError> {
    match owner {
        DocumentOwner::Project(id) => {
            if id != *project {
                return Err(DomainError::NotFound);
            }
            let row = project_row(conn, project)
                .await?
                .ok_or(DomainError::NotFound)?;
            Ok(DocumentOwnerScope {
                archived: row.archived,
                terminal: false,
                task_id: None,
            })
        }
        DocumentOwner::Epic(id) => {
            let scope = epic_scope(conn, project, &id).await?;
            Ok(DocumentOwnerScope {
                archived: scope.ensure_unarchived().is_err(),
                terminal: scope.epic.status.is_terminal(),
                task_id: None,
            })
        }
        DocumentOwner::Task(id) => {
            let scope = task_scope(conn, project, &id).await?;
            Ok(DocumentOwnerScope {
                archived: scope.ensure_unarchived().is_err(),
                terminal: scope.task.status.is_terminal(),
                task_id: Some(id),
            })
        }
    }
}

fn stored_owner(kind: &str, owner_id: &str) -> Result<DocumentOwner, DomainError> {
    let id = parse_uuid("documents.owner_id", owner_id)?;
    match DocumentOwnerKind::parse(kind) {
        Some(DocumentOwnerKind::Project) => Ok(DocumentOwner::Project(ProjectId::from_uuid(id))),
        Some(DocumentOwnerKind::Epic) => {
            Ok(DocumentOwner::Epic(crate::model::EpicId::from_uuid(id)))
        }
        Some(DocumentOwnerKind::Task) => Ok(DocumentOwner::Task(TaskId::from_uuid(id))),
        None => Err(StorageError::Corrupt(format!("documents.owner_kind: {kind}")).into()),
    }
}

fn links_json(links: &[String]) -> Result<String, DomainError> {
    serde_json::to_string(links).map_err(|err| StorageError::Corrupt(err.to_string()).into())
}

const DOCUMENT_COLUMNS: &str = "id, revision, created_at, updated_at, project_id, owner_kind, \
     owner_id, kind, title, latest_revision_id";

fn document_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Document, DomainError> {
    let id: String = row.try_get("id")?;
    let revision: i64 = row.try_get("revision")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let project_id: String = row.try_get("project_id")?;
    let owner_kind: String = row.try_get("owner_kind")?;
    let owner_id: String = row.try_get("owner_id")?;
    let kind: String = row.try_get("kind")?;
    let title: String = row.try_get("title")?;
    let latest_revision_id: String = row.try_get("latest_revision_id")?;
    Ok(Document {
        id: DocumentId::from_uuid(parse_uuid("documents.id", &id)?),
        revision: Revision::from_stored(revision)
            .ok_or_else(|| StorageError::Corrupt(format!("documents.revision: {revision}")))?,
        created_at: parse_ts("documents.created_at", &created_at)?,
        updated_at: parse_ts("documents.updated_at", &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid("documents.project_id", &project_id)?),
        owner_kind: DocumentOwnerKind::parse(&owner_kind)
            .ok_or_else(|| StorageError::Corrupt(format!("documents.owner_kind: {owner_kind}")))?,
        owner_id: parse_uuid("documents.owner_id", &owner_id)?,
        kind: DocumentKind::parse(&kind)
            .ok_or_else(|| StorageError::Corrupt(format!("documents.kind: {kind}")))?,
        title,
        latest_revision_id: DocumentRevisionId::from_uuid(parse_uuid(
            "documents.latest_revision_id",
            &latest_revision_id,
        )?),
    })
}

const REVISION_COLUMNS: &str = "id, document_id, number, body, links, actor_id, created_at";

fn revision_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<DocumentRevision, DomainError> {
    let id: String = row.try_get("id")?;
    let document_id: String = row.try_get("document_id")?;
    let number: i64 = row.try_get("number")?;
    let body: String = row.try_get("body")?;
    let links: String = row.try_get("links")?;
    let actor_id: String = row.try_get("actor_id")?;
    let created_at: String = row.try_get("created_at")?;
    Ok(DocumentRevision {
        id: DocumentRevisionId::from_uuid(parse_uuid("document_revisions.id", &id)?),
        document_id: DocumentId::from_uuid(parse_uuid(
            "document_revisions.document_id",
            &document_id,
        )?),
        number,
        body,
        links: serde_json::from_str(&links)
            .map_err(|err| StorageError::Corrupt(format!("document_revisions.links: {err}")))?,
        actor_id: crate::model::ActorId::from_uuid(parse_uuid(
            "document_revisions.actor_id",
            &actor_id,
        )?),
        created_at: parse_ts("document_revisions.created_at", &created_at)?,
    })
}

async fn document_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    id: &DocumentId,
) -> Result<Option<Document>, DomainError> {
    // AssertSqlSafe: interpolates a compile-time column list only.
    let query = sqlx::AssertSqlSafe(format!(
        "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE id = ?1 AND project_id = ?2"
    ));
    sqlx::query(query)
        .bind(id.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| document_from_row(&row))
        .transpose()
}

async fn revision_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    id: &DocumentRevisionId,
) -> Result<Option<DocumentRevision>, DomainError> {
    sqlx::query(
        "SELECT dr.id, dr.document_id, dr.number, dr.body, dr.links, dr.actor_id, dr.created_at \
         FROM document_revisions dr JOIN documents d ON d.id = dr.document_id \
         WHERE dr.id = ?1 AND d.project_id = ?2",
    )
    .bind(id.to_string())
    .bind(project.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .map(|row| revision_from_row(&row))
    .transpose()
}

#[derive(Debug, Clone, Default)]
pub struct DocumentListFilters {
    pub owner_kind: Option<DocumentOwnerKind>,
    pub owner_id: Option<Uuid>,
    pub kind: Option<DocumentKind>,
}

fn documents_filter(project: &ProjectId, filters: &DocumentListFilters) -> String {
    let mut parts = vec![format!("project={project}")];
    if let Some(owner_kind) = &filters.owner_kind {
        parts.push(format!("owner_kind={}", owner_kind.as_str()));
    }
    if let Some(owner_id) = &filters.owner_id {
        parts.push(format!("owner={owner_id}"));
    }
    if let Some(kind) = &filters.kind {
        parts.push(format!("kind={}", kind.as_str()));
    }
    parts.join("&")
}

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
                    stored_owner(document.owner_kind.as_str(), &document.owner_id.to_string())?,
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

    pub async fn select_task_plan(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task_id: TaskId,
        input: SelectPlanInput,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&ctx.now);

                let scope = task_scope(tx, &project, &task_id).await?;
                let actor = live_actor(tx, &ctx.actor.id).await?;
                require_capability(&actor, Capability::AuthorDocuments)?;
                require_revision(ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if task_execute_or_review_work(tx, task_id, &now_text).await? {
                    return Err(DomainError::ActiveWork(
                        "plan selection is blocked by an execute/review claim or a \
                         pending submission"
                            .into(),
                    ));
                }
                if !scope.task.planning_required {
                    return Err(DomainError::Validation {
                        field: "planning_required",
                        message: "task does not require planning".into(),
                    });
                }
                require_task_plan_revision(tx, &project, &task_id, &input.document_revision_id)
                    .await?;

                let current: Option<String> =
                    sqlx::query_scalar("SELECT selected_plan_revision_id FROM tasks WHERE id = ?1")
                        .bind(task_id.to_string())
                        .fetch_one(&mut **tx)
                        .await?;
                // Re-selecting the current revision changes nothing: acceptance
                // retained, no revision bump, no event (owner decision 2026-10-12).
                if current.as_deref() == Some(input.document_revision_id.to_string().as_str()) {
                    return Ok(CommandResult {
                        value: scope.task,
                        events: Vec::new(),
                    });
                }

                sqlx::query(
                    "UPDATE tasks SET revision = revision + 1, updated_at = ?1, phase = ?2, \
                     selected_plan_revision_id = ?3, accepted_plan_submission_id = NULL \
                     WHERE id = ?4",
                )
                .bind(&now_text)
                .bind(TaskPhase::Planning.as_str())
                .bind(input.document_revision_id.to_string())
                .bind(task_id.to_string())
                .execute(&mut **tx)
                .await?;

                let updated = task_row(tx, &project, &task_id)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "selectTaskPlan",
                    "",
                    vec![PendingEvent::task(task_id, updated.revision, scope.epic.id)],
                )
                .await?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }

    pub async fn get_document(
        &self,
        project: &ProjectId,
        id: &DocumentId,
    ) -> Result<Document, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = document_row(&mut tx, project, id).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn get_document_revision(
        &self,
        project: &ProjectId,
        id: &DocumentRevisionId,
    ) -> Result<DocumentRevision, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = revision_row(&mut tx, project, id).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn list_documents(
        &self,
        project: &ProjectId,
        filters: &DocumentListFilters,
        params: &ListParams,
    ) -> Result<Page<Document>, DomainError> {
        let limit = effective_limit(params)?;
        let filter = documents_filter(project, filters);
        let after = params
            .cursor
            .as_deref()
            .map(|cursor| decode_cursor(cursor, "listDocuments", &filter))
            .transpose()?;

        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE project_id = "
        ));
        builder.push_bind(project.to_string());
        if let Some(owner_kind) = &filters.owner_kind {
            builder
                .push(" AND owner_kind = ")
                .push_bind(owner_kind.as_str());
        }
        if let Some(owner_id) = &filters.owner_id {
            builder
                .push(" AND owner_id = ")
                .push_bind(owner_id.to_string());
        }
        if let Some(kind) = &filters.kind {
            builder.push(" AND kind = ").push_bind(kind.as_str());
        }
        if let Some((created_at, id)) = &after {
            builder
                .push(" AND (created_at > ")
                .push_bind(created_at.clone())
                .push(" OR (created_at = ")
                .push_bind(created_at.clone())
                .push(" AND id > ")
                .push_bind(id.clone())
                .push("))");
        }
        builder
            .push(" ORDER BY created_at ASC, id ASC LIMIT ")
            .push_bind(limit + 1);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(document_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |document: &Document| {
            encode_cursor(
                "listDocuments",
                &filter,
                &format_ts(&document.created_at),
                &document.id.to_string(),
            )
        }))
    }

    pub async fn list_document_revisions(
        &self,
        project: &ProjectId,
        document: &DocumentId,
        params: &ListParams,
    ) -> Result<Page<DocumentRevision>, DomainError> {
        let limit = effective_limit(params)?;
        let filter = format!("project={project}&document={document}");
        let after = params
            .cursor
            .as_deref()
            .map(|cursor| decode_cursor(cursor, "listDocumentRevisions", &filter))
            .transpose()?;

        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        document_row(&mut tx, project, document)
            .await?
            .ok_or(DomainError::NotFound)?;

        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {REVISION_COLUMNS} FROM document_revisions WHERE document_id = "
        ));
        builder.push_bind(document.to_string());
        if let Some((created_at, id)) = &after {
            builder
                .push(" AND (created_at < ")
                .push_bind(created_at.clone())
                .push(" OR (created_at = ")
                .push_bind(created_at.clone())
                .push(" AND id < ")
                .push_bind(id.clone())
                .push("))");
        }
        // Revisions list newest first (plan/07).
        builder
            .push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(limit + 1);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(revision_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |revision: &DocumentRevision| {
            encode_cursor(
                "listDocumentRevisions",
                &filter,
                &format_ts(&revision.created_at),
                &revision.id.to_string(),
            )
        }))
    }
}
