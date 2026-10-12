use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Capability, Document, DocumentCreate, DocumentId, DocumentKind, DocumentOwner,
    DocumentOwnerKind, DocumentRevision, DocumentRevisionId, ProjectId, Revision, RevisionCreate,
    SelectPlanInput, Task, TaskId, TaskPhase, require_capability, validate_document_create,
    validate_revision_create,
};
use crate::queries::hierarchy::{project_exists, project_row, task_row};
use crate::queries::{ListParams, Page, decode_after, effective_limit, encode_cursor, split_page};
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

use super::{
    CommandContext, CommandResult, PendingEvent, Replay, append_events, epic_scope, live_actor,
    missing_after_write, require_revision, task_scope,
};

/// Validate that every UUID in `revision_ids` belongs to a document owned by
/// the task. Duplicate ids are tolerated.
pub(super) async fn validate_task_owned_revisions(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
    revision_ids: &[Uuid],
) -> Result<(), DomainError> {
    let unique: std::collections::HashSet<Uuid> = revision_ids.iter().copied().collect();
    if unique.is_empty() {
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
    for id in &unique {
        separated.push_bind(id.to_string());
    }
    builder.push(")");
    let found: Vec<(String,)> = builder
        .build_query_as()
        .fetch_all(&mut *conn)
        .await
        .map_err(StorageError::from)?;
    if found.len() != unique.len() {
        let found_set: std::collections::HashSet<&str> =
            found.iter().map(|r| r.0.as_str()).collect();
        let missing = unique
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

/// A successful execute report must cite at least one task-owned output
/// revision of kind finding or handoff (plan/04 session rules); ownership is
/// validated separately.
pub(super) async fn require_output_revision(
    conn: &mut SqliteConnection,
    revision_ids: &[Uuid],
) -> Result<(), DomainError> {
    let rejection = || DomainError::Validation {
        field: "document_revision_ids",
        message: "successful execute report requires at least one task-owned \
                  finding or handoff revision"
            .into(),
    };
    if revision_ids.is_empty() {
        return Err(rejection());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT 1 FROM document_revisions dr \
         JOIN documents d ON d.id = dr.document_id \
         WHERE d.kind IN ('finding', 'handoff') AND dr.id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in revision_ids {
        separated.push_bind(id.to_string());
    }
    builder.push(") LIMIT 1");
    let found = builder
        .build()
        .fetch_optional(&mut *conn)
        .await
        .map_err(StorageError::from)?;
    if found.is_none() {
        return Err(rejection());
    }
    Ok(())
}

pub(super) async fn selected_plan_revision(
    conn: &mut SqliteConnection,
    task: TaskId,
) -> Result<Option<Uuid>, DomainError> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT selected_plan_revision_id FROM tasks WHERE id = ?1")
            .bind(task.to_string())
            .fetch_one(&mut *conn)
            .await?;
    stored
        .map(|value| parse_uuid("tasks.selected_plan_revision_id", &value))
        .transpose()
        .map_err(DomainError::from)
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
    let query = sqlx::AssertSqlSafe(format!(
        "SELECT 1 FROM tasks t WHERE t.id = ?1 AND {} LIMIT 1",
        super::claim_work_predicate("?2", Some("'execute', 'review'"))
    ));
    Ok(sqlx::query(query)
        .bind(task.to_string())
        .bind(now)
        .fetch_optional(&mut *conn)
        .await?
        .is_some())
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

                // Authenticate before route membership (plan/04 precedence).
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task_id).await?;
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

                // Re-selecting the current revision changes nothing: acceptance
                // retained, no revision bump, no event (owner decision 2026-10-12).
                if selected_plan_revision(tx, task_id).await? == Some(input.document_revision_id) {
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
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown project is 404.
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        let filter = documents_filter(project, filters);
        let after = decode_after(params, "listDocuments", &filter)?;

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
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown document is 404.
        document_row(&mut tx, project, document)
            .await?
            .ok_or(DomainError::NotFound)?;
        let filter = format!("project={project}&document={document}");
        let after = decode_after(params, "listDocumentRevisions", &filter)?;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use super::*;
    use crate::model::{
        Actor, ActorKind, Clock, CommandId, EpicCreate, EpicId, GoalCreate, ProjectCreate,
        ReviewPolicy, Task, TaskCreate, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{seed_claim, seed_submission, store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        epic: EpicId,
        task: Task,
    }

    fn ctx(actor: &Actor, clock: &TestClock, revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision: revision,
            now: clock.now(),
        }
    }

    async fn fixture(planning_required: bool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind: ActorKind::Human,
            label: "owner".to_string(),
            revoked: false,
            created_at: clock.now(),
        };
        let registered = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();
        let project = store
            .create_project(
                ctx(&owner, &clock, None),
                ProjectCreate {
                    name: "P".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let goal = store
            .create_goal(
                ctx(&owner, &clock, None),
                project,
                GoalCreate {
                    title: "G".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let epic = store
            .create_epic(
                ctx(&owner, &clock, None),
                project,
                goal,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let task = store
            .create_task(
                ctx(&owner, &clock, None),
                project,
                epic,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "code".to_string(),
                    planning_required: Some(planning_required),
                    plan_review: Some(ReviewPolicy::None),
                    work_review: Some(ReviewPolicy::None),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            epic,
            task,
        }
    }

    fn input(owner: DocumentOwner, kind: DocumentKind) -> DocumentCreate {
        DocumentCreate {
            owner,
            kind,
            title: "Doc".to_string(),
            body: "# body".to_string(),
            links: Vec::new(),
        }
    }

    async fn doc(f: &Fixture, owner: DocumentOwner, kind: DocumentKind) -> Document {
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .create_document(
                ctx(&f.owner, &f.clock, None),
                f.project,
                input(owner, kind),
                "h",
            )
            .await
            .unwrap()
            .into_inner()
    }

    async fn append(
        f: &Fixture,
        document: &Document,
        revision: Option<i64>,
        body: &str,
    ) -> Result<DocumentRevision, DomainError> {
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .create_document_revision(
                ctx(&f.owner, &f.clock, revision),
                f.project,
                document.id,
                RevisionCreate {
                    body: body.to_string(),
                    links: Vec::new(),
                },
                "rev-h",
            )
            .await
            .map(Replay::into_inner)
    }

    async fn select(f: &Fixture, revision_id: Uuid) -> Result<Task, DomainError> {
        f.clock.advance(TimeDelta::milliseconds(2));
        let current: i64 = sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
            .bind(f.task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        f.store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(current)),
                f.project,
                f.task.id,
                SelectPlanInput {
                    document_revision_id: revision_id,
                },
            )
            .await
            .map(|result| result.value)
    }

    async fn plan_state(f: &Fixture) -> (Option<String>, Option<String>, String) {
        sqlx::query_as(
            "SELECT selected_plan_revision_id, accepted_plan_submission_id, phase \
             FROM tasks WHERE id = ?1",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn documents_are_created_for_all_owner_kinds() {
        let f = fixture(false).await;
        for (owner, kind) in [
            (DocumentOwner::Project(f.project), DocumentKind::Brief),
            (DocumentOwner::Epic(f.epic), DocumentKind::Decision),
            (DocumentOwner::Task(f.task.id), DocumentKind::Plan),
        ] {
            let document = doc(&f, owner, kind).await;
            assert_eq!(document.revision, Revision::INITIAL);
            assert_eq!(document.owner_kind, owner.kind());
            assert_eq!(document.owner_id, owner.id());
            let revision = f
                .store
                .get_document_revision(&f.project, &document.latest_revision_id)
                .await
                .unwrap();
            assert_eq!(revision.number, 1);
        }
    }

    #[tokio::test]
    async fn unknown_owners_are_not_found() {
        let f = fixture(false).await;
        let other_project = ProjectId::generate(f.clock.now());
        for owner in [
            DocumentOwner::Project(other_project),
            DocumentOwner::Epic(EpicId::generate(f.clock.now())),
            DocumentOwner::Task(TaskId::generate(f.clock.now())),
        ] {
            let err = f
                .store
                .create_document(
                    ctx(&f.owner, &f.clock, None),
                    f.project,
                    input(owner, DocumentKind::Note),
                    "h",
                )
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::NotFound), "{owner:?}");
        }
    }

    #[tokio::test]
    async fn invalid_content_is_rejected_by_the_command() {
        let f = fixture(false).await;
        let cases = [
            DocumentCreate {
                title: String::new(),
                ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
            },
            DocumentCreate {
                body: "x".repeat(65_537),
                ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
            },
            DocumentCreate {
                links: vec!["javascript:alert(1)".to_string()],
                ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
            },
        ];
        for bad in cases {
            let err = f
                .store
                .create_document(ctx(&f.owner, &f.clock, None), f.project, bad, "h")
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::Validation { .. }));
        }
        let documents: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(documents, 0);
    }

    #[tokio::test]
    async fn appends_bump_the_document_and_enforce_if_match() {
        let f = fixture(false).await;
        let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        let second = append(&f, &document, Some(1), "v2").await.unwrap();
        assert_eq!(second.number, 2);
        let reloaded = f
            .store
            .get_document(&f.project, &document.id)
            .await
            .unwrap();
        assert_eq!(reloaded.revision.value(), 2);
        assert_eq!(reloaded.latest_revision_id, second.id);

        let err = append(&f, &document, Some(1), "stale").await.unwrap_err();
        assert!(matches!(
            err,
            DomainError::RevisionConflict {
                expected: 1,
                actual: 2
            }
        ));
        let err = append(&f, &document, None, "no precondition")
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PreconditionRequired));
        let err = append(&f, &document, Some(2), "x".repeat(65_537).as_str())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "body", .. }));
    }

    #[tokio::test]
    async fn terminal_owner_accepts_only_notes() {
        let f = fixture(false).await;
        let finding = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), f.project, f.task.id)
            .await
            .unwrap();

        let note = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        assert!(append(&f, &note, Some(1), "still fine").await.is_ok());

        let err = f
            .store
            .create_document(
                ctx(&f.owner, &f.clock, None),
                f.project,
                input(DocumentOwner::Task(f.task.id), DocumentKind::Plan),
                "h",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
        let err = append(&f, &finding, Some(1), "late").await.unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn archived_scope_rejects_creates_appends_and_selection() {
        let f = fixture(true).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                f.task.id,
                "obsolete".into(),
            )
            .await
            .unwrap();
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                f.project,
                f.task.id,
                "cleanup".into(),
            )
            .await
            .unwrap();

        let err = f
            .store
            .create_document(
                ctx(&f.owner, &f.clock, None),
                f.project,
                input(DocumentOwner::Task(f.task.id), DocumentKind::Note),
                "h",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = append(&f, &plan, Some(1), "late").await.unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = select(&f, plan.latest_revision_id.as_uuid())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn execute_claims_block_selected_plan_appends_only() {
        let f = fixture(true).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        let notes = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        select(&f, plan.latest_revision_id.as_uuid()).await.unwrap();

        let now = format_ts(&f.clock.now());
        let expires = format_ts(&(f.clock.now() + TimeDelta::seconds(300)));
        seed_claim(
            f.store.pool(),
            f.task.id,
            f.owner.id,
            "execute",
            "active",
            &now,
            &expires,
        )
        .await;

        let err = append(&f, &plan, Some(1), "v2").await.unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
        assert!(append(&f, &notes, Some(1), "fine").await.is_ok());

        // An expired lease no longer blocks.
        f.clock.advance(TimeDelta::seconds(1_000));
        assert!(append(&f, &plan, Some(1), "v2").await.is_ok());
    }

    #[tokio::test]
    async fn selection_persists_clears_acceptance_and_noops_on_same_revision() {
        let f = fixture(true).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        let v1 = plan.latest_revision_id.as_uuid();
        let v2 = append(&f, &plan, Some(1), "v2").await.unwrap().id.as_uuid();

        let updated = select(&f, v1).await.unwrap();
        assert_eq!(updated.phase, crate::model::TaskPhase::Planning);
        let (selected, _, _) = plan_state(&f).await;
        assert_eq!(selected.as_deref(), Some(v1.to_string().as_str()));

        // Simulate an accepted plan, then change the selection: acceptance clears.
        let now = format_ts(&f.clock.now());
        let submission =
            seed_submission(f.store.pool(), f.task.id, f.owner.id, "accepted", &now).await;
        sqlx::query(
            "UPDATE tasks SET accepted_plan_submission_id = ?1, phase = 'execution' WHERE id = ?2",
        )
        .bind(&submission)
        .bind(f.task.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let updated = select(&f, v2).await.unwrap();
        assert_eq!(updated.phase, crate::model::TaskPhase::Planning);
        let (selected, accepted, phase) = plan_state(&f).await;
        assert_eq!(selected.as_deref(), Some(v2.to_string().as_str()));
        assert_eq!(accepted, None);
        assert_eq!(phase, "planning");

        // Re-selecting the same revision changes nothing and emits no event.
        let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        let revision_before = updated.revision;
        let noop = select(&f, v2).await.unwrap();
        assert_eq!(noop.revision, revision_before);
        let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(events_after, events_before);
    }

    #[tokio::test]
    async fn selection_guards_reject_invalid_targets_and_active_work() {
        let f = fixture(true).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        let note = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        let v1 = plan.latest_revision_id.as_uuid();

        let err = select(&f, note.latest_revision_id.as_uuid())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "document_revision_id",
                ..
            }
        ));
        let err = select(&f, Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "document_revision_id",
                ..
            }
        ));

        // Stale If-Match and missing precondition.
        f.clock.advance(TimeDelta::milliseconds(2));
        let err = f
            .store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(99)),
                f.project,
                f.task.id,
                SelectPlanInput {
                    document_revision_id: v1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::RevisionConflict { .. }));
        let err = f
            .store
            .select_task_plan(
                ctx(&f.owner, &f.clock, None),
                f.project,
                f.task.id,
                SelectPlanInput {
                    document_revision_id: v1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PreconditionRequired));

        // Unknown task is a 404.
        let err = f
            .store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                TaskId::generate(f.clock.now()),
                SelectPlanInput {
                    document_revision_id: v1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        // Pending submissions and review claims block selection.
        let now = format_ts(&f.clock.now());
        let submission =
            seed_submission(f.store.pool(), f.task.id, f.owner.id, "pending", &now).await;
        let err = select(&f, v1).await.unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
        sqlx::query("DELETE FROM submissions WHERE id = ?1")
            .bind(&submission)
            .execute(f.store.pool())
            .await
            .unwrap();
        let expires = format_ts(&(f.clock.now() + TimeDelta::seconds(300)));
        seed_claim(
            f.store.pool(),
            f.task.id,
            f.owner.id,
            "review",
            "active",
            &now,
            &expires,
        )
        .await;
        let err = select(&f, v1).await.unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
    }

    #[tokio::test]
    async fn selection_requires_planning() {
        let f = fixture(false).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        let err = select(&f, plan.latest_revision_id.as_uuid())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "planning_required",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn revoked_actors_cannot_select() {
        let f = fixture(true).await;
        let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
        sqlx::query("UPDATE actors SET revoked = 1 WHERE id = ?1")
            .bind(f.owner.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let err = select(&f, plan.latest_revision_id.as_uuid())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[tokio::test]
    async fn reads_are_project_scoped_and_lists_filter_and_paginate() {
        let f = fixture(false).await;
        let task_doc = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        let epic_doc = doc(&f, DocumentOwner::Epic(f.epic), DocumentKind::Brief).await;
        let other_project = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "Other".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;

        assert!(matches!(
            f.store.get_document(&other_project, &task_doc.id).await,
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            f.store
                .get_document_revision(&other_project, &task_doc.latest_revision_id)
                .await,
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            f.store
                .list_document_revisions(&other_project, &task_doc.id, &ListParams::default())
                .await,
            Err(DomainError::NotFound)
        ));

        let by_owner = f
            .store
            .list_documents(
                &f.project,
                &DocumentListFilters {
                    owner_kind: Some(DocumentOwnerKind::Epic),
                    owner_id: Some(f.epic.as_uuid()),
                    kind: Some(DocumentKind::Brief),
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(by_owner.items.len(), 1);
        assert_eq!(by_owner.items[0].id, epic_doc.id);

        let page_one = f
            .store
            .list_documents(
                &f.project,
                &DocumentListFilters::default(),
                &ListParams {
                    limit: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page_one.items.len(), 1);
        assert_eq!(page_one.items[0].id, task_doc.id, "created_at ascending");
        let cursor = page_one.next_cursor.expect("two documents exceed the page");
        let page_two = f
            .store
            .list_documents(
                &f.project,
                &DocumentListFilters::default(),
                &ListParams {
                    limit: Some(1),
                    cursor: Some(cursor.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page_two.items[0].id, epic_doc.id);
        let err = f
            .store
            .list_documents(
                &f.project,
                &DocumentListFilters {
                    kind: Some(DocumentKind::Note),
                    ..Default::default()
                },
                &ListParams {
                    cursor: Some(cursor),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidCursor(_)));

        append(&f, &task_doc, Some(1), "v2").await.unwrap();
        append(&f, &task_doc, Some(2), "v3").await.unwrap();
        let newest_first = f
            .store
            .list_document_revisions(
                &f.project,
                &task_doc.id,
                &ListParams {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let numbers: Vec<i64> = newest_first.items.iter().map(|r| r.number).collect();
        assert_eq!(numbers, vec![3, 2]);
        let rest = f
            .store
            .list_document_revisions(
                &f.project,
                &task_doc.id,
                &ListParams {
                    limit: Some(2),
                    cursor: newest_first.next_cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let numbers: Vec<i64> = rest.items.iter().map(|r| r.number).collect();
        assert_eq!(numbers, vec![1]);
    }

    #[tokio::test]
    async fn duplicate_owned_revision_ids_validate() {
        let f = fixture(false).await;
        let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
        let owned = document.latest_revision_id.as_uuid();
        let project = f.project;
        let task_id = f.task.id;
        f.store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    validate_task_owned_revisions(tx, &project, &task_id, &[owned, owned]).await
                })
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn revoked_actors_get_forbidden_before_membership() {
        let f = fixture(true).await;
        sqlx::query("UPDATE actors SET revoked = 1 WHERE id = ?1")
            .bind(f.owner.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        // A revoked actor must not learn whether a task id exists.
        let err = f
            .store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                TaskId::generate(f.clock.now()),
                SelectPlanInput {
                    document_revision_id: Uuid::nil(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[tokio::test]
    async fn unknown_project_lists_are_not_found_before_cursor_checks() {
        let f = fixture(false).await;
        let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
        let unknown = ProjectId::generate(f.clock.now());
        let garbage_cursor = ListParams {
            cursor: Some("not-a-cursor".into()),
            ..Default::default()
        };
        let err = f
            .store
            .list_documents(
                &unknown,
                &DocumentListFilters::default(),
                &ListParams::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .list_documents(&unknown, &DocumentListFilters::default(), &garbage_cursor)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::NotFound),
            "membership precedes cursor validity"
        );
        let err = f
            .store
            .list_document_revisions(&unknown, &document.id, &garbage_cursor)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::NotFound),
            "membership precedes cursor validity"
        );
    }

    #[tokio::test]
    async fn task_owned_revision_validation_reports_the_missing_id() {
        let f = fixture(false).await;
        let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
        let owned = document.latest_revision_id.as_uuid();
        let foreign = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let project = f.project;
        let task_id = f.task.id;
        let err = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    validate_task_owned_revisions(tx, &project, &task_id, &[owned, foreign]).await
                })
            })
            .await
            .unwrap_err();
        match err {
            DomainError::Validation { field, message } => {
                assert_eq!(field, "document_revision_ids");
                assert!(message.contains(&foreign.to_string()));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }
}
