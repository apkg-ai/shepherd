//! Async SQLite storage layer.
//!
//! Wraps an `sqlx::SqlitePool` and provides domain-aware CRUD operations.
//! All invariant enforcement (lifecycle, DAG, lease) is delegated to the
//! pure-logic modules and enforced within transactions here.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::bundle;
use crate::dag;
use crate::error::Error;
use crate::export;
use crate::lease;
use crate::lifecycle::{self, Trigger};
use crate::model::*;

type Result<T> = std::result::Result<T, Error>;

/// SQLite-backed domain store.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Create a store from an existing pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Access the underlying connection pool (useful for raw SQL in tests).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Open a file-backed store at the given path. Creates the file and
    /// runs migrations if it does not yet exist.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;

        let store = Self::new(pool);
        store.migrate().await?;
        Ok(store)
    }

    /// Create an in-memory store (for tests). Runs migrations automatically.
    pub async fn new_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;

        let store = Self::new(pool);
        store.migrate().await?;
        Ok(store)
    }

    /// Run all pending migrations.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("migration failed: {e}")))?;
        Ok(())
    }

    // ── Projects ─────────────────────────────────────────────────────────

    pub async fn create_project(&self, input: &ProjectCreate) -> Result<Project> {
        let id = ProjectId::new();
        let now = Utc::now();
        let desc = input.description.as_deref().unwrap_or("");
        let review_gate = input
            .settings
            .as_ref()
            .map(|s| s.review_gate)
            .unwrap_or(true);

        sqlx::query(
            "INSERT INTO projects (id, name, description, review_gate, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&input.name)
        .bind(desc)
        .bind(review_gate)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.get_project(id).await
    }

    pub async fn get_project(&self, id: ProjectId) -> Result<Project> {
        let row = sqlx::query(
            "SELECT id, name, description, review_gate, deleted_at, created_at, updated_at
             FROM projects WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("project", id))?;

        row_to_project(&row)
    }

    pub async fn update_project(&self, id: ProjectId, input: &ProjectUpdate) -> Result<Project> {
        let existing = self.get_project(id).await?;
        let now = Utc::now();

        let name = input.name.as_deref().unwrap_or(&existing.name);
        let desc = input
            .description
            .as_deref()
            .unwrap_or(&existing.description);
        let review_gate = input
            .settings
            .as_ref()
            .map(|s| s.review_gate)
            .unwrap_or(existing.settings.review_gate);

        sqlx::query(
            "UPDATE projects SET name = ?, description = ?, review_gate = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(name)
        .bind(desc)
        .bind(review_gate)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_project(id).await
    }

    pub async fn delete_project(&self, id: ProjectId) -> Result<()> {
        let _ = self.get_project(id).await?;
        let now = Utc::now();

        // Cascade: soft-delete all non-deleted tasks in this project
        // (which in turn releases their claims and triggers auto-ready).
        let task_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM tasks WHERE project_id = ? AND deleted_at IS NULL")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        for tid_str in &task_ids {
            let tid: TaskId = tid_str
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            // Use internal delete to avoid re-checking project existence.
            self.delete_task_internal(id, tid, now).await?;
        }

        sqlx::query("UPDATE projects SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn list_projects(&self, cursor: Option<&str>, limit: i64) -> Result<Page<Project>> {
        let limit = limit.clamp(1, 100);

        let rows = if let Some(c) = cursor {
            let c = Cursor::decode(c)?;
            sqlx::query(
                "SELECT id, name, description, review_gate, deleted_at, created_at, updated_at
                 FROM projects
                 WHERE deleted_at IS NULL
                   AND (created_at, id) < (?, ?)
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?",
            )
            .bind(c.created_at.to_rfc3339())
            .bind(c.id.to_string())
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, name, description, review_gate, deleted_at, created_at, updated_at
                 FROM projects
                 WHERE deleted_at IS NULL
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?",
            )
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        };

        let items: Vec<Project> = rows.iter().map(row_to_project).collect::<Result<_>>()?;

        Ok(Page::from_rows(items, limit as usize, |p| Cursor {
            created_at: p.created_at,
            id: p.id.0,
        }))
    }

    // ── Tasks ────────────────────────────────────────────────────────────

    pub async fn create_task(&self, project_id: ProjectId, input: &TaskCreate) -> Result<Task> {
        let _ = self.get_project(project_id).await?;

        // Validate initial status.
        let status = input.status.unwrap_or(TaskStatus::Proposed);
        if status != TaskStatus::Proposed && status != TaskStatus::Approved {
            return Err(Error::ValidationError {
                detail: "initial status must be proposed or approved".into(),
                errors: vec![],
            });
        }

        // Validate metadata.
        let metadata = input
            .metadata
            .as_ref()
            .cloned()
            .unwrap_or(serde_json::json!({}));
        validate_metadata(&metadata)?;

        let id = TaskId::new();
        let now = Utc::now();
        let desc = input.description.as_deref().unwrap_or("");
        let assignee_json = input
            .assignee
            .as_ref()
            .map(|a| serde_json::to_string(a).unwrap());
        let graph_role = input.graph_role.as_deref().unwrap_or(&[]);
        let graph_role_json = serde_json::to_string(graph_role).unwrap();
        let graph_role_explicit = input.graph_role.is_some();

        // If approved and no deps, auto-ready.
        let final_status = if status == TaskStatus::Approved {
            // Check if all dependencies are done (new task has none, so
            // it's immediately ready).
            TaskStatus::Ready
        } else {
            status
        };

        sqlx::query(
            "INSERT INTO tasks (id, project_id, title, description, type, status, metadata,
                                assignee, graph_role, graph_role_explicit, attempt_count,
                                created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(project_id.to_string())
        .bind(&input.title)
        .bind(desc)
        .bind(input.task_type.to_string())
        .bind(final_status.to_string())
        .bind(serde_json::to_string(&metadata).unwrap())
        .bind(assignee_json.as_deref())
        .bind(&graph_role_json)
        .bind(graph_role_explicit)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Derive graph roles if not explicitly set.
        if !graph_role_explicit {
            self.recompute_graph_roles(project_id).await?;
        }

        self.get_task(project_id, id).await
    }

    pub async fn get_task(&self, project_id: ProjectId, id: TaskId) -> Result<Task> {
        let row = sqlx::query(
            "SELECT id, project_id, title, description, type, status, metadata,
                    assignee, graph_role, graph_role_explicit, attempt_count,
                    blocked_from_status, block_reason, deleted_at, created_at, updated_at
             FROM tasks
             WHERE id = ? AND project_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("task", id))?;

        row_to_task(&row)
    }

    pub async fn update_task(
        &self,
        project_id: ProjectId,
        id: TaskId,
        input: &TaskUpdate,
    ) -> Result<Task> {
        let existing = self.get_task(project_id, id).await?;
        let now = Utc::now();

        let title = input.title.as_deref().unwrap_or(&existing.title);
        let desc = input
            .description
            .as_deref()
            .unwrap_or(&existing.description);
        let task_type = input.task_type.unwrap_or(existing.task_type);

        let metadata = if let Some(m) = &input.metadata {
            validate_metadata(m)?;
            m.clone()
        } else {
            existing.metadata.clone()
        };

        let assignee = match &input.assignee {
            Some(a) => a.as_ref(),
            None => existing.assignee.as_ref(),
        };
        let assignee_json = assignee.map(|a| serde_json::to_string(a).unwrap());

        let (graph_role, graph_role_explicit) = if let Some(roles) = &input.graph_role {
            (roles.clone(), true)
        } else {
            (existing.graph_role.clone(), false) // preserve existing
        };
        let graph_role_json = serde_json::to_string(&graph_role).unwrap();

        sqlx::query(
            "UPDATE tasks SET title = ?, description = ?, type = ?, metadata = ?,
                              assignee = ?, graph_role = ?, graph_role_explicit = ?,
                              updated_at = ?
             WHERE id = ? AND project_id = ? AND deleted_at IS NULL",
        )
        .bind(title)
        .bind(desc)
        .bind(task_type.to_string())
        .bind(serde_json::to_string(&metadata).unwrap())
        .bind(assignee_json.as_deref())
        .bind(&graph_role_json)
        .bind(graph_role_explicit)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(project_id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_task(project_id, id).await
    }

    pub async fn delete_task(&self, project_id: ProjectId, id: TaskId) -> Result<()> {
        let _ = self.get_task(project_id, id).await?;
        self.delete_task_internal(project_id, id, Utc::now()).await
    }

    /// Internal task soft-delete with cascade. Shared by `delete_task` and
    /// the project-cascade path in `delete_project`.
    async fn delete_task_internal(
        &self,
        project_id: ProjectId,
        id: TaskId,
        now: DateTime<Utc>,
    ) -> Result<()> {
        // Cascade: release any active claim.
        if let Some(claim) = self.get_active_claim(id, now).await? {
            self.release_claim_internal(claim.id, now, "task_deleted")
                .await?;
        }

        // Find dependents BEFORE soft-deleting (so edges are still visible).
        let dependent_ids: Vec<String> = sqlx::query_scalar(
            "SELECT source_task_id FROM relations
             WHERE type = 'depends_on' AND target_task_id = ?",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        // Soft-delete the task.
        sqlx::query(
            "UPDATE tasks SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND project_id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(project_id.to_string())
        .execute(&self.pool)
        .await?;

        // Recompute graph roles since edges involving this task are now
        // effectively removed.
        self.recompute_graph_roles(project_id).await?;

        // Auto-ready cascade: dependents that had a depends_on to this
        // (now invisible) task may become unblocked.
        for dep_str in &dependent_ids {
            let dep_id: TaskId = dep_str
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            self.try_auto_ready(project_id, dep_id).await?;
        }

        Ok(())
    }

    pub async fn list_tasks(
        &self,
        project_id: ProjectId,
        cursor: Option<&str>,
        limit: i64,
        status: Option<TaskStatus>,
        task_type: Option<TaskType>,
    ) -> Result<Page<Task>> {
        let limit = limit.clamp(1, 100);

        // Build dynamic query with optional filters.
        let mut sql = String::from(
            "SELECT id, project_id, title, description, type, status, metadata,
                    assignee, graph_role, graph_role_explicit, attempt_count,
                    blocked_from_status, block_reason, deleted_at, created_at, updated_at
             FROM tasks
             WHERE project_id = ? AND deleted_at IS NULL",
        );
        let mut binds: Vec<String> = vec![project_id.to_string()];

        if let Some(s) = status {
            sql.push_str(" AND status = ?");
            binds.push(s.to_string());
        }
        if let Some(t) = task_type {
            sql.push_str(" AND type = ?");
            binds.push(t.to_string());
        }
        if let Some(c) = cursor {
            let c = Cursor::decode(c)?;
            sql.push_str(" AND (created_at, id) < (?, ?)");
            binds.push(c.created_at.to_rfc3339());
            binds.push(c.id.to_string());
        }

        sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
        binds.push((limit + 1).to_string());

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query.fetch_all(&self.pool).await?;
        let items: Vec<Task> = rows.iter().map(row_to_task).collect::<Result<_>>()?;

        Ok(Page::from_rows(items, limit as usize, |t| Cursor {
            created_at: t.created_at,
            id: t.id.0,
        }))
    }

    /// Get the highest-priority ready and unclaimed task.
    ///
    /// Priority: tasks that unblock the most downstream work. Tie-break:
    /// oldest `created_at`, then lowest `id`.
    pub async fn next_task(&self, project_id: ProjectId) -> Result<Option<Task>> {
        let now = Utc::now();

        // Lazy sweep: return expired-lease tasks to `ready` before selecting,
        // so a crashed agent's task is offered again immediately.
        self.sweep_expired_claims(now).await?;

        // Find all ready tasks with no active claim.
        let rows = sqlx::query(
            "SELECT t.id, t.project_id, t.title, t.description, t.type, t.status,
                    t.metadata, t.assignee, t.graph_role, t.graph_role_explicit,
                    t.attempt_count, t.blocked_from_status, t.block_reason,
                    t.deleted_at, t.created_at, t.updated_at
             FROM tasks t
             WHERE t.project_id = ?
               AND t.status = 'ready'
               AND t.deleted_at IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM claims c
                   WHERE c.task_id = t.id
                     AND c.released_at IS NULL
                     AND c.expires_at > ?
               )
             ORDER BY t.created_at ASC, t.id ASC",
        )
        .bind(project_id.to_string())
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let tasks: Vec<Task> = rows.iter().map(row_to_task).collect::<Result<_>>()?;

        if tasks.len() == 1 {
            return Ok(Some(tasks.into_iter().next().unwrap()));
        }

        // Score each by downstream dependent count.
        let edges = self.load_depends_on_edges(project_id).await?;
        let mut best: Option<(Task, usize)> = None;

        for task in tasks {
            let score = dag::count_downstream(task.id, &edges);
            match &best {
                None => best = Some((task, score)),
                Some((_, best_score)) if score > *best_score => {
                    best = Some((task, score));
                }
                _ => {} // keep existing (earlier created_at wins by query order)
            }
        }

        Ok(best.map(|(t, _)| t))
    }

    // ── Task actions ─────────────────────────────────────────────────────

    pub async fn approve_task(&self, project_id: ProjectId, id: TaskId) -> Result<Task> {
        let task = self.get_task(project_id, id).await?;
        // One endpoint, two review moments (03-api.md): approving a proposal
        // and approving work in review.
        let trigger = if task.status == TaskStatus::InReview {
            Trigger::HumanApproval
        } else {
            Trigger::Approve
        };
        let new_status = lifecycle::transition(task.status, &trigger)?;

        self.set_task_status(project_id, id, new_status).await?;

        match new_status {
            // Auto-ready: check if all deps are done.
            TaskStatus::Approved => self.try_auto_ready(project_id, id).await?,
            // Review approval completed the task — ready its dependents.
            TaskStatus::Done => self.auto_ready_cascade(project_id, id).await?,
            _ => {}
        }

        self.get_task(project_id, id).await
    }

    pub async fn reject_task(
        &self,
        project_id: ProjectId,
        id: TaskId,
        _reason: &str,
    ) -> Result<Task> {
        let task = self.get_task(project_id, id).await?;
        let mut new_status = lifecycle::transition(task.status, &Trigger::HumanRejection)?;

        // Dependencies may have been added while in review; a rejected task
        // only returns to ready when they are all done (invariant 4).
        if new_status == TaskStatus::Ready && !self.all_deps_done(project_id, id).await? {
            new_status = TaskStatus::Approved;
        }

        self.set_task_status(project_id, id, new_status).await?;

        self.get_task(project_id, id).await
    }

    pub async fn block_task(
        &self,
        project_id: ProjectId,
        id: TaskId,
        reason: &str,
    ) -> Result<Task> {
        let task = self.get_task(project_id, id).await?;
        let _ = lifecycle::transition(task.status, &Trigger::Block)?;
        let now = Utc::now();

        sqlx::query(
            "UPDATE tasks SET status = 'blocked', blocked_from_status = ?,
                              block_reason = ?, updated_at = ?
             WHERE id = ? AND project_id = ?",
        )
        .bind(task.status.to_string())
        .bind(reason)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(project_id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_task(project_id, id).await
    }

    pub async fn unblock_task(&self, project_id: ProjectId, id: TaskId) -> Result<Task> {
        let task = self.get_task(project_id, id).await?;
        let blocked_from = task
            .blocked_from_status
            .ok_or_else(|| Error::InvalidTransition {
                from: task.status,
                trigger: "Unblock".into(),
                detail: "task has no blocked_from_status".into(),
            })?;

        let mut new_status =
            lifecycle::transition(task.status, &Trigger::Unblock { blocked_from })?;

        // The lease may have expired while blocked. Restoring to InProgress
        // without an active claim would strand the task (session reports
        // require the claim), so demote to Ready and let it be re-claimed.
        if new_status == TaskStatus::InProgress
            && self.get_active_claim(id, Utc::now()).await?.is_none()
        {
            new_status = TaskStatus::Ready;
        }

        // Dependencies may have been added while blocked. If restoring to
        // Ready but deps aren't all done, demote to Approved instead.
        if new_status == TaskStatus::Ready && !self.all_deps_done(project_id, id).await? {
            new_status = TaskStatus::Approved;
        }

        // Dependencies may also have *completed* while blocked — the
        // auto-ready cascade skips blocked tasks. Promote so the task does
        // not sit approved-with-deps-done forever (invariant 4).
        if new_status == TaskStatus::Approved && self.all_deps_done(project_id, id).await? {
            new_status = TaskStatus::Ready;
        }

        let now = Utc::now();
        sqlx::query(
            "UPDATE tasks SET status = ?, blocked_from_status = NULL,
                              block_reason = NULL, updated_at = ?
             WHERE id = ? AND project_id = ?",
        )
        .bind(new_status.to_string())
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(project_id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_task(project_id, id).await
    }

    pub async fn cancel_task(&self, project_id: ProjectId, id: TaskId) -> Result<Task> {
        let task = self.get_task(project_id, id).await?;
        let new_status = lifecycle::transition(task.status, &Trigger::Cancel)?;

        self.set_task_status(project_id, id, new_status).await?;

        self.get_task(project_id, id).await
    }

    // ── Relations ────────────────────────────────────────────────────────

    pub async fn create_relation(
        &self,
        project_id: ProjectId,
        source_task_id: TaskId,
        input: &RelationCreate,
    ) -> Result<Relation> {
        // Verify both tasks exist and belong to the project.
        let source = self.get_task(project_id, source_task_id).await?;
        let _ = self.get_task(project_id, input.target_task_id).await?;

        if source_task_id == input.target_task_id {
            return Err(Error::ValidationError {
                detail: "a task cannot relate to itself".into(),
                errors: vec![],
            });
        }

        match input.relation_type {
            RelationType::DependsOn => {
                // Cycle detection.
                let edges = self.load_depends_on_edges(project_id).await?;
                if dag::would_create_cycle(&edges, source_task_id, input.target_task_id) {
                    return Err(Error::DependencyCycle {
                        detail: format!(
                            "adding depends_on from {} to {} would create a cycle",
                            source_task_id, input.target_task_id
                        ),
                    });
                }

                // Ready demotion: if source is Ready and new dep is not Done,
                // demote to Approved.
                if source.status == TaskStatus::Ready {
                    let target = self.get_task(project_id, input.target_task_id).await?;
                    if target.status != TaskStatus::Done {
                        self.set_task_status(project_id, source_task_id, TaskStatus::Approved)
                            .await?;
                    }
                }
            }
            RelationType::Decomposition => {
                // Single-parent check.
                let decomp_edges = self.load_decomposition_edges(project_id).await?;
                if dag::would_violate_single_parent(&decomp_edges, input.target_task_id) {
                    return Err(Error::DecompositionViolation {
                        detail: format!(
                            "task {} already has a decomposition parent",
                            input.target_task_id
                        ),
                    });
                }
            }
        }

        let id = RelationId::new();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(input.relation_type.to_string())
        .bind(source_task_id.to_string())
        .bind(input.target_task_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Recompute graph roles.
        self.recompute_graph_roles(project_id).await?;

        self.get_relation(id).await
    }

    pub async fn delete_relation(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        id: RelationId,
    ) -> Result<()> {
        let rel = self.get_relation(id).await?;

        // Verify the relation involves the given task.
        if rel.source_task_id != task_id && rel.target_task_id != task_id {
            return Err(Error::not_found("relation", id));
        }

        sqlx::query("DELETE FROM relations WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        // Recompute graph roles.
        self.recompute_graph_roles(project_id).await?;

        // Auto-ready cascade: removing a dependency might unblock tasks.
        if rel.relation_type == RelationType::DependsOn {
            self.try_auto_ready(project_id, rel.source_task_id).await?;
        }

        Ok(())
    }

    pub async fn list_relations(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<Vec<Relation>> {
        let _ = self.get_task(project_id, task_id).await?;

        let rows = sqlx::query(
            "SELECT id, type, source_task_id, target_task_id, created_at
             FROM relations
             WHERE source_task_id = ? OR target_task_id = ?",
        )
        .bind(task_id.to_string())
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_relation).collect()
    }

    // ── Claims ───────────────────────────────────────────────────────────

    pub async fn claim_task(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        input: &ClaimRequest,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        lease::validate_ttl(input.ttl_seconds)?;

        let task = self.get_task(project_id, task_id).await?;

        // Claim conflict outranks status: a claimed task is also not ready,
        // but the caller should learn someone else holds the lease.
        if let Some(existing) = self.get_active_claim(task_id, now).await? {
            return Err(Error::ClaimConflict {
                detail: format!(
                    "task {} is already claimed (claim {}, expires {})",
                    task_id, existing.id, existing.expires_at
                ),
            });
        }

        if task.status != TaskStatus::Ready {
            return Err(Error::TaskNotReady {
                detail: format!("task {} is in {} status, not ready", task_id, task.status),
            });
        }

        let claim_id = ClaimId::new();
        let lease_id = lease::generate_lease_id();
        let expires_at = lease::compute_expiry(now, input.ttl_seconds);
        let new_status = lifecycle::transition(task.status, &Trigger::Claim)?;

        // All claim-path writes run in one transaction so concurrent attempts
        // serialize on SQLite's write lock; the partial unique index on
        // claims(task_id) WHERE released_at IS NULL is the final arbiter.
        let mut tx = self.pool.begin().await?;

        // Release expired-unswept claims for this task — they still occupy
        // the unique-index slot. This first write also takes the write lock,
        // serializing concurrent claim attempts.
        sqlx::query(
            "UPDATE claims SET released_at = ?, release_reason = 'expired'
             WHERE task_id = ? AND released_at IS NULL AND expires_at <= ?",
        )
        .bind(now.to_rfc3339())
        .bind(task_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;

        // Friendly conflict check (the unique index catches any race).
        let existing = sqlx::query(
            "SELECT id, expires_at FROM claims
             WHERE task_id = ? AND released_at IS NULL",
        )
        .bind(task_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing {
            return Err(Error::ClaimConflict {
                detail: format!(
                    "task {} is already claimed (claim {}, expires {})",
                    task_id,
                    row.get::<String, _>("id"),
                    row.get::<String, _>("expires_at")
                ),
            });
        }

        // Guarded status flip: re-verifies `ready` under the write lock.
        let flipped = sqlx::query(
            "UPDATE tasks SET status = ?, assignee = ?, updated_at = ?
             WHERE id = ? AND project_id = ? AND status = 'ready'
               AND deleted_at IS NULL",
        )
        .bind(new_status.to_string())
        .bind(serde_json::to_string(&input.identity).unwrap())
        .bind(now.to_rfc3339())
        .bind(task_id.to_string())
        .bind(project_id.to_string())
        .execute(&mut *tx)
        .await?;
        if flipped.rows_affected() != 1 {
            return Err(Error::TaskNotReady {
                detail: format!("task {task_id} is no longer ready"),
            });
        }

        let inserted = sqlx::query(
            "INSERT INTO claims (id, task_id, identity, ttl_seconds, lease_id,
                                 acquired_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(claim_id.to_string())
        .bind(task_id.to_string())
        .bind(serde_json::to_string(&input.identity).unwrap())
        .bind(input.ttl_seconds)
        .bind(&lease_id)
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {}
            Err(e)
                if e.as_database_error()
                    .is_some_and(|d| d.is_unique_violation()) =>
            {
                return Err(Error::ClaimConflict {
                    detail: format!("task {task_id} is already claimed"),
                });
            }
            Err(e) => return Err(e.into()),
        }

        tx.commit().await?;

        self.get_claim(claim_id).await
    }

    pub async fn renew_claim(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        input: &ClaimRenewal,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        let _ = self.get_task(project_id, task_id).await?;

        let claim =
            self.get_active_claim(task_id, now)
                .await?
                .ok_or_else(|| Error::LeaseExpired {
                    detail: format!("no active claim on task {task_id}"),
                })?;

        if !claim.identity.matches(&input.identity) {
            return Err(Error::ClaimConflict {
                detail: "claim identity mismatch".into(),
            });
        }

        let ttl = input.ttl_seconds.unwrap_or(claim.ttl_seconds);
        lease::validate_ttl(ttl)?;
        let new_expires = lease::compute_renewal_expiry(now, ttl);

        sqlx::query(
            "UPDATE claims SET expires_at = ?, renewed_at = ?, ttl_seconds = ?
             WHERE id = ?",
        )
        .bind(new_expires.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(ttl)
        .bind(claim.id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_claim(claim.id).await
    }

    pub async fn release_claim(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        input: &ClaimRelease,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let task = self.get_task(project_id, task_id).await?;

        let claim =
            self.get_active_claim(task_id, now)
                .await?
                .ok_or_else(|| Error::LeaseExpired {
                    detail: format!("no active claim on task {task_id}"),
                })?;

        if !claim.identity.matches(&input.identity) {
            return Err(Error::ClaimConflict {
                detail: "claim identity mismatch".into(),
            });
        }

        self.release_claim_internal(claim.id, now, "voluntary")
            .await?;

        // Return task to ready (or approved if new deps were added).
        if task.status == TaskStatus::InProgress {
            let target = if self.all_deps_done(project_id, task_id).await? {
                TaskStatus::Ready
            } else {
                TaskStatus::Approved
            };
            self.set_task_status(project_id, task_id, target).await?;
        }

        Ok(())
    }

    /// Sweep expired claims and return the task IDs that were released.
    pub async fn sweep_expired_claims(&self, now: DateTime<Utc>) -> Result<Vec<TaskId>> {
        let rows = sqlx::query(
            "SELECT c.id, c.task_id, t.project_id, t.status
             FROM claims c
             JOIN tasks t ON t.id = c.task_id
             WHERE c.released_at IS NULL AND c.expires_at <= ?",
        )
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        let mut released = Vec::new();

        for row in &rows {
            let claim_id: ClaimId = row
                .get::<String, _>("id")
                .parse()
                .map(ClaimId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            let task_id: TaskId = row
                .get::<String, _>("task_id")
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            let project_id: ProjectId = row
                .get::<String, _>("project_id")
                .parse()
                .map(ProjectId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            let status: TaskStatus = row
                .get::<String, _>("status")
                .parse()
                .map_err(|e: String| Error::Internal(e))?;

            self.release_claim_internal(claim_id, now, "expired")
                .await?;

            if status == TaskStatus::InProgress {
                // Check deps — new ones may have been added while claimed.
                let target = if self.all_deps_done(project_id, task_id).await? {
                    TaskStatus::Ready
                } else {
                    TaskStatus::Approved
                };
                self.set_task_status(project_id, task_id, target).await?;
            }

            released.push(task_id);
        }

        // Crash recovery: an `in_progress` task with no unreleased claim row
        // is unreachable (claiming needs `ready`, reporting needs the claim).
        // The state can only arise from a crash between a claim release and
        // its status update; rescue it back to ready/approved. The grace
        // period keeps the rescue from racing a healthy release→status
        // window in release_claim/create_session.
        let cutoff = now - TimeDelta::seconds(30);
        let orphans = sqlx::query(
            "SELECT t.id, t.project_id FROM tasks t
             WHERE t.status = 'in_progress'
               AND t.deleted_at IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM claims c
                   WHERE c.task_id = t.id
                     AND (c.released_at IS NULL OR c.released_at > ?)
               )",
        )
        .bind(cutoff.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        for row in &orphans {
            let task_id: TaskId = row
                .get::<String, _>("id")
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            let project_id: ProjectId = row
                .get::<String, _>("project_id")
                .parse()
                .map(ProjectId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;

            let target = if self.all_deps_done(project_id, task_id).await? {
                TaskStatus::Ready
            } else {
                TaskStatus::Approved
            };
            self.set_task_status(project_id, task_id, target).await?;
            released.push(task_id);
        }

        Ok(released)
    }

    // ── Sessions ─────────────────────────────────────────────────────────

    pub async fn create_session(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        input: &SessionReport,
    ) -> Result<Session> {
        let task = self.get_task(project_id, task_id).await?;

        if task.status != TaskStatus::InProgress {
            return Err(Error::InvalidTransition {
                from: task.status,
                trigger: "SessionReport".into(),
                detail: "task must be in_progress to report a session".into(),
            });
        }

        let now = Utc::now();

        // Strict claim guard: the reporter must hold the active claim.
        // Prevents a stale claimant (expired lease, task since re-claimed)
        // from releasing someone else's claim and advancing the task.
        let claim =
            self.get_active_claim(task_id, now)
                .await?
                .ok_or_else(|| Error::LeaseExpired {
                    detail: format!(
                        "no active claim on task {task_id}; the lease may have expired — \
                         re-claim the task before reporting"
                    ),
                })?;
        if !claim.identity.matches(&input.identity) {
            return Err(Error::ClaimConflict {
                detail: "session identity does not match the active claim".into(),
            });
        }
        self.release_claim_internal(claim.id, now, "session_reported")
            .await?;

        let session_id = SessionId::new();
        let decisions = input.decisions.as_deref().unwrap_or(&[]);
        let artifacts = input.artifacts.as_deref().unwrap_or(&[]);

        sqlx::query(
            "INSERT INTO sessions (id, task_id, identity, started_at, ended_at,
                                   outcome, failure_reason, summary, decisions,
                                   artifacts, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(task_id.to_string())
        .bind(serde_json::to_string(&input.identity).unwrap())
        .bind(input.started_at.to_rfc3339())
        .bind(input.ended_at.to_rfc3339())
        .bind(input.outcome.to_string())
        .bind(input.failure_reason.as_deref())
        .bind(input.summary.as_deref().unwrap_or(""))
        .bind(serde_json::to_string(decisions).unwrap())
        .bind(serde_json::to_string(artifacts).unwrap())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Create knowledge items from the report.
        if let Some(ki_inputs) = &input.knowledge_items {
            for ki_input in ki_inputs {
                self.create_knowledge_internal(
                    project_id,
                    ki_input,
                    Some(task_id),
                    Some(session_id),
                )
                .await?;
            }
        }

        // Increment attempt count.
        sqlx::query(
            "UPDATE tasks SET attempt_count = attempt_count + 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await?;

        // Transition task based on outcome.
        match input.outcome {
            SessionOutcome::Succeeded => {
                let project = self.get_project(project_id).await?;
                let trigger = Trigger::SessionSuccess {
                    review_gate: project.settings.review_gate,
                };
                let new_status = lifecycle::transition(task.status, &trigger)?;
                self.set_task_status(project_id, task_id, new_status)
                    .await?;

                // Auto-ready cascade if task reached Done.
                if new_status == TaskStatus::Done {
                    self.auto_ready_cascade(project_id, task_id).await?;
                }
            }
            SessionOutcome::Failed => {
                // Task returns to ready/approved (failure is session
                // outcome, not task state). Check deps because new ones
                // may have been added while the task was in_progress.
                let target = if self.all_deps_done(project_id, task_id).await? {
                    TaskStatus::Ready
                } else {
                    TaskStatus::Approved
                };
                self.set_task_status(project_id, task_id, target).await?;
            }
        }

        self.get_session(project_id, task_id, session_id).await
    }

    pub async fn get_session(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        id: SessionId,
    ) -> Result<Session> {
        let _ = self.get_task(project_id, task_id).await?;

        let row = sqlx::query(
            "SELECT id, task_id, identity, started_at, ended_at, outcome,
                    failure_reason, summary, decisions, artifacts, created_at
             FROM sessions
             WHERE id = ? AND task_id = ?",
        )
        .bind(id.to_string())
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("session", id))?;

        let mut session = row_to_session(&row)?;
        session.knowledge_items = self.load_session_knowledge(session.id).await?;
        Ok(session)
    }

    pub async fn list_sessions(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Page<Session>> {
        let _ = self.get_task(project_id, task_id).await?;
        let limit = limit.clamp(1, 100);

        let rows = if let Some(c) = cursor {
            let c = Cursor::decode(c)?;
            sqlx::query(
                "SELECT id, task_id, identity, started_at, ended_at, outcome,
                        failure_reason, summary, decisions, artifacts, created_at
                 FROM sessions
                 WHERE task_id = ? AND (created_at, id) < (?, ?)
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?",
            )
            .bind(task_id.to_string())
            .bind(c.created_at.to_rfc3339())
            .bind(c.id.to_string())
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, task_id, identity, started_at, ended_at, outcome,
                        failure_reason, summary, decisions, artifacts, created_at
                 FROM sessions
                 WHERE task_id = ?
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?",
            )
            .bind(task_id.to_string())
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        };

        let mut items: Vec<Session> = rows.iter().map(row_to_session).collect::<Result<_>>()?;
        for session in &mut items {
            session.knowledge_items = self.load_session_knowledge(session.id).await?;
        }

        Ok(Page::from_rows(items, limit as usize, |s| Cursor {
            created_at: s.created_at,
            id: s.id.0,
        }))
    }

    /// Load the knowledge items produced during a session.
    async fn load_session_knowledge(&self, session_id: SessionId) -> Result<Vec<KnowledgeItem>> {
        let rows = sqlx::query(
            "SELECT id, type, title, content, scope, task_id, session_id,
                    project_id, created_at
             FROM knowledge_items
             WHERE session_id = ?
             ORDER BY created_at ASC, id ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_knowledge).collect()
    }

    // ── Knowledge ────────────────────────────────────────────────────────

    pub async fn create_knowledge(
        &self,
        project_id: ProjectId,
        input: &KnowledgeItemCreate,
    ) -> Result<KnowledgeItem> {
        self.create_knowledge_internal(project_id, input, None, None)
            .await
    }

    pub async fn get_knowledge(
        &self,
        project_id: ProjectId,
        id: KnowledgeId,
    ) -> Result<KnowledgeItem> {
        let row = sqlx::query(
            "SELECT id, type, title, content, scope, task_id, session_id,
                    project_id, created_at
             FROM knowledge_items
             WHERE id = ? AND project_id = ?",
        )
        .bind(id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("knowledge item", id))?;

        row_to_knowledge(&row)
    }

    pub async fn delete_knowledge(&self, project_id: ProjectId, id: KnowledgeId) -> Result<()> {
        let _ = self.get_knowledge(project_id, id).await?;

        sqlx::query("DELETE FROM knowledge_items WHERE id = ? AND project_id = ?")
            .bind(id.to_string())
            .bind(project_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn list_knowledge(
        &self,
        project_id: ProjectId,
        cursor: Option<&str>,
        limit: i64,
        scope: Option<KnowledgeScope>,
        knowledge_type: Option<KnowledgeType>,
        task_id: Option<TaskId>,
    ) -> Result<Page<KnowledgeItem>> {
        let limit = limit.clamp(1, 100);

        let mut sql = String::from(
            "SELECT id, type, title, content, scope, task_id, session_id,
                    project_id, created_at
             FROM knowledge_items
             WHERE project_id = ?",
        );
        let mut binds: Vec<String> = vec![project_id.to_string()];

        if let Some(s) = scope {
            sql.push_str(" AND scope = ?");
            binds.push(s.to_string());
        }
        if let Some(t) = knowledge_type {
            sql.push_str(" AND type = ?");
            binds.push(t.to_string());
        }
        if let Some(tid) = task_id {
            sql.push_str(" AND task_id = ?");
            binds.push(tid.to_string());
        }
        if let Some(c) = cursor {
            let c = Cursor::decode(c)?;
            sql.push_str(" AND (created_at, id) < (?, ?)");
            binds.push(c.created_at.to_rfc3339());
            binds.push(c.id.to_string());
        }

        sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
        binds.push((limit + 1).to_string());

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query.fetch_all(&self.pool).await?;
        let items: Vec<KnowledgeItem> = rows.iter().map(row_to_knowledge).collect::<Result<_>>()?;

        Ok(Page::from_rows(items, limit as usize, |k| Cursor {
            created_at: k.created_at,
            id: k.id.0,
        }))
    }

    // ── Context bundle ───────────────────────────────────────────────────

    pub async fn get_context_bundle(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<ContextBundle> {
        let task = self.get_task(project_id, task_id).await?;

        // Load edges.
        let deps = self.load_depends_on_edges(project_id).await?;
        let decomp = self.load_decomposition_edges(project_id).await?;

        // Walk ancestors.
        let ancestor_ids = dag::ancestor_chain(task_id, &deps, &decomp);

        let mut ancestors_with_sessions = Vec::new();
        for aid in &ancestor_ids {
            // Ancestors might be in other projects (shouldn't happen, but
            // be safe).
            if let Ok(ancestor) = self.get_task(project_id, *aid).await {
                let sessions = self.list_all_sessions_for_task(*aid).await?;
                ancestors_with_sessions.push((ancestor, sessions));
            }
        }

        // Project knowledge.
        let project_knowledge = self
            .list_knowledge_by_scope(project_id, KnowledgeScope::Project)
            .await?;

        // In-flight siblings: other in_progress tasks with active claims.
        let now = Utc::now();
        let sibling_rows = sqlx::query(
            "SELECT t.id, t.project_id, t.title, t.description, t.type, t.status,
                    t.metadata, t.assignee, t.graph_role, t.graph_role_explicit,
                    t.attempt_count, t.blocked_from_status, t.block_reason,
                    t.deleted_at, t.created_at, t.updated_at,
                    c.identity as claim_identity
             FROM tasks t
             JOIN claims c ON c.task_id = t.id
             WHERE t.project_id = ?
               AND t.status = 'in_progress'
               AND t.deleted_at IS NULL
               AND c.released_at IS NULL
               AND c.expires_at > ?",
        )
        .bind(project_id.to_string())
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        let mut sibling_claims = Vec::new();
        for row in &sibling_rows {
            let t = row_to_task(row)?;
            let identity_json: String = row.get("claim_identity");
            let identity: Identity = serde_json::from_str(&identity_json)
                .map_err(|e| Error::Internal(format!("bad identity JSON: {e}")))?;
            sibling_claims.push((t, identity));
        }

        Ok(bundle::assemble(
            task,
            ancestors_with_sessions,
            project_knowledge,
            sibling_claims,
        ))
    }

    // ── Export / Import ──────────────────────────────────────────────────

    pub async fn export_project(&self, project_id: ProjectId) -> Result<ExportDocument> {
        let project = self.get_project(project_id).await?;

        let tasks = self.list_all_tasks(project_id).await?;
        let relations = self.list_all_relations(project_id).await?;
        let sessions = self.list_all_sessions(project_id).await?;
        let knowledge = self.list_all_knowledge(project_id).await?;

        Ok(export::build_export(
            project, tasks, relations, sessions, knowledge,
        ))
    }

    pub async fn import_project(&self, doc: &ExportDocument) -> Result<ImportResult> {
        export::validate_import(doc)?;

        // Create new project with fresh IDs.
        let new_project_id = ProjectId::new();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO projects (id, name, description, review_gate, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(new_project_id.to_string())
        .bind(&doc.project.name)
        .bind(&doc.project.description)
        .bind(doc.project.settings.review_gate)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Build old → new ID mappings.
        let mut task_map: HashMap<TaskId, TaskId> = HashMap::new();
        let mut session_map: HashMap<SessionId, SessionId> = HashMap::new();

        // Claims are runtime state and are not exported, so an imported
        // `in_progress` task could never advance (reports require an active
        // claim). Normalize: `in_progress` → `ready` when all its deps in
        // the document are `done`, else `approved`. `approved` with all deps
        // `done` also normalizes to `ready` — nothing re-runs the auto-ready
        // check after import, and invariant 4 (`ready` ⇔ `approved` + deps
        // done) must hold in the new project.
        let doc_status: HashMap<TaskId, TaskStatus> =
            doc.tasks.iter().map(|t| (t.id, t.status)).collect();
        let normalized_status = |task: &Task| -> TaskStatus {
            if task.status != TaskStatus::InProgress && task.status != TaskStatus::Approved {
                return task.status;
            }
            let deps_done = doc
                .relations
                .iter()
                .filter(|r| {
                    r.relation_type == RelationType::DependsOn && r.source_task_id == task.id
                })
                .all(|r| doc_status.get(&r.target_task_id) == Some(&TaskStatus::Done));
            if deps_done {
                TaskStatus::Ready
            } else {
                TaskStatus::Approved
            }
        };

        // Import tasks.
        for task in &doc.tasks {
            let new_id = TaskId::new();
            task_map.insert(task.id, new_id);

            let assignee_json = task
                .assignee
                .as_ref()
                .map(|a| serde_json::to_string(a).unwrap());
            let graph_role_json = serde_json::to_string(&task.graph_role).unwrap();

            // The wire schema does not carry blocked_from_status, so REST
            // imports of blocked tasks arrive without it — and unblock would
            // reject them forever. Fall back to `ready`: unblock re-checks
            // deps and claims and demotes as needed.
            let blocked_from = match (task.status, task.blocked_from_status) {
                (TaskStatus::Blocked, None) => Some(TaskStatus::Ready),
                (_, bf) => bf,
            };

            sqlx::query(
                "INSERT INTO tasks (id, project_id, title, description, type, status,
                                    metadata, assignee, graph_role, graph_role_explicit,
                                    attempt_count, blocked_from_status, block_reason,
                                    created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id.to_string())
            .bind(new_project_id.to_string())
            .bind(&task.title)
            .bind(&task.description)
            .bind(task.task_type.to_string())
            .bind(normalized_status(task).to_string())
            .bind(serde_json::to_string(&task.metadata).unwrap())
            .bind(assignee_json.as_deref())
            .bind(&graph_role_json)
            .bind(true) // preserve explicit roles from export
            .bind(task.attempt_count)
            .bind(blocked_from.map(|s| s.to_string()))
            .bind(task.block_reason.as_deref())
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        // Import relations.
        for rel in &doc.relations {
            let new_source = task_map[&rel.source_task_id];
            let new_target = task_map[&rel.target_task_id];

            sqlx::query(
                "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(RelationId::new().to_string())
            .bind(rel.relation_type.to_string())
            .bind(new_source.to_string())
            .bind(new_target.to_string())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        // Import sessions.
        for session in &doc.sessions {
            let new_id = SessionId::new();
            let new_task_id = task_map[&session.task_id];
            session_map.insert(session.id, new_id);

            sqlx::query(
                "INSERT INTO sessions (id, task_id, identity, started_at, ended_at,
                                       outcome, failure_reason, summary, decisions,
                                       artifacts, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id.to_string())
            .bind(new_task_id.to_string())
            .bind(serde_json::to_string(&session.identity).unwrap())
            .bind(session.started_at.to_rfc3339())
            .bind(session.ended_at.to_rfc3339())
            .bind(session.outcome.to_string())
            .bind(session.failure_reason.as_deref())
            .bind(&session.summary)
            .bind(serde_json::to_string(&session.decisions).unwrap())
            .bind(serde_json::to_string(&session.artifacts).unwrap())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        // Import knowledge items.
        for ki in &doc.knowledge_items {
            let new_task_id = ki.task_id.map(|id| task_map[&id]);
            let new_session_id = ki.session_id.map(|id| session_map[&id]);

            sqlx::query(
                "INSERT INTO knowledge_items (id, type, title, content, scope,
                                              task_id, session_id, project_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(KnowledgeId::new().to_string())
            .bind(ki.knowledge_type.to_string())
            .bind(&ki.title)
            .bind(&ki.content)
            .bind(ki.scope.to_string())
            .bind(new_task_id.map(|id| id.to_string()))
            .bind(new_session_id.map(|id| id.to_string()))
            .bind(new_project_id.to_string())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        Ok(export::import_summary(
            new_project_id,
            doc.tasks.len(),
            doc.relations.len(),
            doc.sessions.len(),
            doc.knowledge_items.len(),
        ))
    }

    // ── Purge (hard delete) ─────────────────────────────────────────────

    /// Permanently remove a soft-deleted project and all its children.
    /// Returns an error if the project is not soft-deleted.
    pub async fn purge_project(&self, id: ProjectId) -> Result<()> {
        // Verify the project exists AND is soft-deleted.
        let row = sqlx::query("SELECT deleted_at FROM projects WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::not_found("project", id))?;

        let deleted_at: Option<String> = row.get("deleted_at");
        if deleted_at.is_none() {
            return Err(Error::ValidationError {
                detail: "cannot purge a project that is not soft-deleted".into(),
                errors: vec![],
            });
        }

        // Load ALL task IDs (including soft-deleted) in this project.
        let task_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM tasks WHERE project_id = ?")
            .bind(id.to_string())
            .fetch_all(&self.pool)
            .await?;

        // Delete children in FK-safe order.
        for tid in &task_ids {
            sqlx::query("DELETE FROM knowledge_items WHERE task_id = ?")
                .bind(tid)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE task_id = ?")
                .bind(tid)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM claims WHERE task_id = ?")
                .bind(tid)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM relations WHERE source_task_id = ? OR target_task_id = ?")
                .bind(tid)
                .bind(tid)
                .execute(&self.pool)
                .await?;
        }

        // Delete project-scoped knowledge (task_id IS NULL).
        sqlx::query("DELETE FROM knowledge_items WHERE project_id = ? AND task_id IS NULL")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM tasks WHERE project_id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Purge all soft-deleted projects and tasks older than `before`.
    pub async fn purge_all_deleted(&self, before: DateTime<Utc>) -> Result<PurgeResult> {
        let mut projects_purged = 0usize;
        let mut tasks_purged = 0usize;

        // Purge soft-deleted projects older than the threshold.
        let project_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM projects WHERE deleted_at IS NOT NULL AND deleted_at < ?",
        )
        .bind(before.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        for pid_str in &project_ids {
            let pid: ProjectId = pid_str
                .parse()
                .map(ProjectId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            self.purge_project(pid).await?;
            projects_purged += 1;
        }

        // Purge orphaned soft-deleted tasks (project still alive).
        let task_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, project_id FROM tasks
             WHERE deleted_at IS NOT NULL AND deleted_at < ?
               AND project_id IN (SELECT id FROM projects WHERE deleted_at IS NULL)",
        )
        .bind(before.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        for (tid_str, _pid_str) in &task_rows {
            // Delete children in FK-safe order.
            sqlx::query("DELETE FROM knowledge_items WHERE task_id = ?")
                .bind(tid_str)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE task_id = ?")
                .bind(tid_str)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM claims WHERE task_id = ?")
                .bind(tid_str)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM relations WHERE source_task_id = ? OR target_task_id = ?")
                .bind(tid_str)
                .bind(tid_str)
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM tasks WHERE id = ?")
                .bind(tid_str)
                .execute(&self.pool)
                .await?;
            tasks_purged += 1;
        }

        Ok(PurgeResult {
            projects_purged,
            tasks_purged,
        })
    }

    // ── Internal helpers ─────────────────────────────────────────────────

    async fn set_task_status(
        &self,
        project_id: ProjectId,
        id: TaskId,
        status: TaskStatus,
    ) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE tasks SET status = ?, updated_at = ?
             WHERE id = ? AND project_id = ? AND deleted_at IS NULL",
        )
        .bind(status.to_string())
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(project_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Check if a task can be auto-readied (approved + all deps done).
    async fn try_auto_ready(&self, project_id: ProjectId, task_id: TaskId) -> Result<()> {
        let task = self.get_task(project_id, task_id).await?;
        if task.status != TaskStatus::Approved {
            return Ok(());
        }

        if self.all_deps_done(project_id, task_id).await? {
            let new_status = lifecycle::transition(task.status, &Trigger::AutoReady)?;
            self.set_task_status(project_id, task_id, new_status)
                .await?;
        }

        Ok(())
    }

    /// When a task reaches Done, scan its dependents and auto-ready those
    /// whose deps are now all done.
    async fn auto_ready_cascade(
        &self,
        project_id: ProjectId,
        completed_task_id: TaskId,
    ) -> Result<()> {
        // Find tasks that depend on the completed task.
        let rows = sqlx::query(
            "SELECT source_task_id FROM relations
             WHERE type = 'depends_on' AND target_task_id = ?",
        )
        .bind(completed_task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        for row in &rows {
            let dep_id: TaskId = row
                .get::<String, _>("source_task_id")
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;

            self.try_auto_ready(project_id, dep_id).await?;
        }

        Ok(())
    }

    /// Check if all `depends_on` targets of a task are Done.
    async fn all_deps_done(&self, _project_id: ProjectId, task_id: TaskId) -> Result<bool> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM relations r
             JOIN tasks t ON t.id = r.target_task_id
             WHERE r.source_task_id = ?
               AND r.type = 'depends_on'
               AND t.deleted_at IS NULL
               AND t.status != 'done'",
        )
        .bind(task_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        let undone: i64 = row.get("cnt");
        Ok(undone == 0)
    }

    /// Read-only: an expired claim is simply not active. Expired rows are
    /// released only by the claim path (in-transaction) and the sweep — the
    /// two places that also restore the task status. Releasing here would
    /// orphan the task in `in_progress` with no row left for the sweep.
    async fn get_active_claim(&self, task_id: TaskId, now: DateTime<Utc>) -> Result<Option<Claim>> {
        let row = sqlx::query(
            "SELECT id, task_id, identity, ttl_seconds, lease_id,
                    acquired_at, expires_at, renewed_at
             FROM claims
             WHERE task_id = ? AND released_at IS NULL AND expires_at > ?",
        )
        .bind(task_id.to_string())
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(row_to_claim(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_claim(&self, id: ClaimId) -> Result<Claim> {
        let row = sqlx::query(
            "SELECT id, task_id, identity, ttl_seconds, lease_id,
                    acquired_at, expires_at, renewed_at
             FROM claims WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("claim", id))?;

        row_to_claim(&row)
    }

    async fn get_relation(&self, id: RelationId) -> Result<Relation> {
        let row = sqlx::query(
            "SELECT id, type, source_task_id, target_task_id, created_at
             FROM relations WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::not_found("relation", id))?;

        row_to_relation(&row)
    }

    async fn release_claim_internal(
        &self,
        claim_id: ClaimId,
        now: DateTime<Utc>,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE claims SET released_at = ?, release_reason = ?
             WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(reason)
        .bind(claim_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn create_knowledge_internal(
        &self,
        project_id: ProjectId,
        input: &KnowledgeItemCreate,
        task_id_override: Option<TaskId>,
        session_id_override: Option<SessionId>,
    ) -> Result<KnowledgeItem> {
        let _ = self.get_project(project_id).await?;
        let id = KnowledgeId::new();
        let now = Utc::now();

        let task_id = task_id_override.or(input.task_id);
        let session_id = session_id_override.or(input.session_id);

        sqlx::query(
            "INSERT INTO knowledge_items (id, type, title, content, scope,
                                          task_id, session_id, project_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(input.knowledge_type.to_string())
        .bind(&input.title)
        .bind(&input.content)
        .bind(input.scope.to_string())
        .bind(task_id.map(|t| t.to_string()))
        .bind(session_id.map(|s| s.to_string()))
        .bind(project_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.get_knowledge(project_id, id).await
    }

    async fn load_depends_on_edges(&self, project_id: ProjectId) -> Result<Vec<(TaskId, TaskId)>> {
        let rows = sqlx::query(
            "SELECT r.source_task_id, r.target_task_id
             FROM relations r
             JOIN tasks t1 ON t1.id = r.source_task_id AND t1.deleted_at IS NULL
             JOIN tasks t2 ON t2.id = r.target_task_id AND t2.deleted_at IS NULL
             WHERE r.type = 'depends_on'
               AND t1.project_id = ?",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let src: TaskId = row
                    .get::<String, _>("source_task_id")
                    .parse()
                    .map(TaskId::from_uuid)
                    .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
                let tgt: TaskId = row
                    .get::<String, _>("target_task_id")
                    .parse()
                    .map(TaskId::from_uuid)
                    .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
                Ok((src, tgt))
            })
            .collect()
    }

    async fn load_decomposition_edges(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<(TaskId, TaskId)>> {
        let rows = sqlx::query(
            "SELECT r.source_task_id, r.target_task_id
             FROM relations r
             JOIN tasks t1 ON t1.id = r.source_task_id AND t1.deleted_at IS NULL
             JOIN tasks t2 ON t2.id = r.target_task_id AND t2.deleted_at IS NULL
             WHERE r.type = 'decomposition'
               AND t1.project_id = ?",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let src: TaskId = row
                    .get::<String, _>("source_task_id")
                    .parse()
                    .map(TaskId::from_uuid)
                    .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
                let tgt: TaskId = row
                    .get::<String, _>("target_task_id")
                    .parse()
                    .map(TaskId::from_uuid)
                    .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
                Ok((src, tgt))
            })
            .collect()
    }

    async fn recompute_graph_roles(&self, project_id: ProjectId) -> Result<()> {
        // Load all non-deleted task IDs in this project.
        let rows = sqlx::query(
            "SELECT id, graph_role_explicit FROM tasks
             WHERE project_id = ? AND deleted_at IS NULL",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut non_explicit_ids = Vec::new();
        let mut all_ids = Vec::new();

        for row in &rows {
            let id: TaskId = row
                .get::<String, _>("id")
                .parse()
                .map(TaskId::from_uuid)
                .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;
            let explicit: bool = row.get("graph_role_explicit");
            all_ids.push(id);
            if !explicit {
                non_explicit_ids.push(id);
            }
        }

        if non_explicit_ids.is_empty() {
            return Ok(());
        }

        let edges = self.load_depends_on_edges(project_id).await?;
        let derived = dag::derive_graph_roles(&all_ids, &edges);
        let now = Utc::now();

        for id in &non_explicit_ids {
            let roles = derived.get(id).cloned().unwrap_or_default();
            let json = serde_json::to_string(&roles).unwrap();

            sqlx::query(
                "UPDATE tasks SET graph_role = ?, updated_at = ?
                 WHERE id = ? AND graph_role_explicit = 0",
            )
            .bind(&json)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    // Bulk loaders for export.

    async fn list_all_tasks(&self, project_id: ProjectId) -> Result<Vec<Task>> {
        let rows = sqlx::query(
            "SELECT id, project_id, title, description, type, status, metadata,
                    assignee, graph_role, graph_role_explicit, attempt_count,
                    blocked_from_status, block_reason, deleted_at, created_at, updated_at
             FROM tasks
             WHERE project_id = ? AND deleted_at IS NULL
             ORDER BY created_at ASC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_task).collect()
    }

    async fn list_all_relations(&self, project_id: ProjectId) -> Result<Vec<Relation>> {
        let rows = sqlx::query(
            "SELECT r.id, r.type, r.source_task_id, r.target_task_id, r.created_at
             FROM relations r
             JOIN tasks t ON t.id = r.source_task_id AND t.deleted_at IS NULL
             WHERE t.project_id = ?
             ORDER BY r.created_at ASC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_relation).collect()
    }

    async fn list_all_sessions(&self, project_id: ProjectId) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT s.id, s.task_id, s.identity, s.started_at, s.ended_at,
                    s.outcome, s.failure_reason, s.summary, s.decisions,
                    s.artifacts, s.created_at
             FROM sessions s
             JOIN tasks t ON t.id = s.task_id AND t.deleted_at IS NULL
             WHERE t.project_id = ?
             ORDER BY s.created_at ASC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_session).collect()
    }

    async fn list_all_sessions_for_task(&self, task_id: TaskId) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, task_id, identity, started_at, ended_at, outcome,
                    failure_reason, summary, decisions, artifacts, created_at
             FROM sessions
             WHERE task_id = ?
             ORDER BY created_at ASC",
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_session).collect()
    }

    async fn list_all_knowledge(&self, project_id: ProjectId) -> Result<Vec<KnowledgeItem>> {
        let rows = sqlx::query(
            "SELECT id, type, title, content, scope, task_id, session_id,
                    project_id, created_at
             FROM knowledge_items
             WHERE project_id = ?
             ORDER BY created_at ASC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_knowledge).collect()
    }

    async fn list_knowledge_by_scope(
        &self,
        project_id: ProjectId,
        scope: KnowledgeScope,
    ) -> Result<Vec<KnowledgeItem>> {
        let rows = sqlx::query(
            "SELECT id, type, title, content, scope, task_id, session_id,
                    project_id, created_at
             FROM knowledge_items
             WHERE project_id = ? AND scope = ?
             ORDER BY created_at ASC",
        )
        .bind(project_id.to_string())
        .bind(scope.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_knowledge).collect()
    }
}

// ── Row → struct helpers ─────────────────────────────────────────────────

fn parse_dt(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad datetime: {e}")))
}

fn parse_optional_dt(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    match s {
        Some(v) => Ok(Some(parse_dt(&v)?)),
        None => Ok(None),
    }
}

fn row_to_project(row: &sqlx::sqlite::SqliteRow) -> Result<Project> {
    Ok(Project {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(ProjectId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        name: row.get("name"),
        description: row.get("description"),
        settings: ProjectSettings {
            review_gate: row.get("review_gate"),
        },
        deleted_at: parse_optional_dt(row.get("deleted_at"))?,
        created_at: parse_dt(&row.get::<String, _>("created_at"))?,
        updated_at: parse_dt(&row.get::<String, _>("updated_at"))?,
    })
}

fn row_to_task(row: &sqlx::sqlite::SqliteRow) -> Result<Task> {
    let assignee: Option<Identity> = row
        .get::<Option<String>, _>("assignee")
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| Error::Internal(format!("bad assignee JSON: {e}")))?;

    let graph_role: Vec<GraphRole> = serde_json::from_str(&row.get::<String, _>("graph_role"))
        .map_err(|e| Error::Internal(format!("bad graph_role JSON: {e}")))?;

    let metadata: serde_json::Value = serde_json::from_str(&row.get::<String, _>("metadata"))
        .map_err(|e| Error::Internal(format!("bad metadata JSON: {e}")))?;

    let blocked_from_status: Option<TaskStatus> = row
        .get::<Option<String>, _>("blocked_from_status")
        .map(|s| s.parse::<TaskStatus>())
        .transpose()
        .map_err(|e| Error::Internal(format!("bad blocked_from_status: {e}")))?;

    Ok(Task {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(TaskId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        project_id: row
            .get::<String, _>("project_id")
            .parse()
            .map(ProjectId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        title: row.get("title"),
        description: row.get("description"),
        task_type: row
            .get::<String, _>("type")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        status: row
            .get::<String, _>("status")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        metadata,
        assignee,
        graph_role,
        attempt_count: row.get("attempt_count"),
        blocked_from_status,
        block_reason: row.get("block_reason"),
        deleted_at: parse_optional_dt(row.get("deleted_at"))?,
        created_at: parse_dt(&row.get::<String, _>("created_at"))?,
        updated_at: parse_dt(&row.get::<String, _>("updated_at"))?,
    })
}

fn row_to_relation(row: &sqlx::sqlite::SqliteRow) -> Result<Relation> {
    Ok(Relation {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(RelationId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        relation_type: row
            .get::<String, _>("type")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        source_task_id: row
            .get::<String, _>("source_task_id")
            .parse()
            .map(TaskId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        target_task_id: row
            .get::<String, _>("target_task_id")
            .parse()
            .map(TaskId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        created_at: parse_dt(&row.get::<String, _>("created_at"))?,
    })
}

fn row_to_claim(row: &sqlx::sqlite::SqliteRow) -> Result<Claim> {
    let identity: Identity = serde_json::from_str(&row.get::<String, _>("identity"))
        .map_err(|e| Error::Internal(format!("bad identity JSON: {e}")))?;

    Ok(Claim {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(ClaimId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        task_id: row
            .get::<String, _>("task_id")
            .parse()
            .map(TaskId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        identity,
        ttl_seconds: row.get("ttl_seconds"),
        lease_id: row.get("lease_id"),
        acquired_at: parse_dt(&row.get::<String, _>("acquired_at"))?,
        expires_at: parse_dt(&row.get::<String, _>("expires_at"))?,
        renewed_at: parse_optional_dt(row.get("renewed_at"))?,
    })
}

fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> Result<Session> {
    let identity: Identity = serde_json::from_str(&row.get::<String, _>("identity"))
        .map_err(|e| Error::Internal(format!("bad identity JSON: {e}")))?;

    let decisions: Vec<String> = serde_json::from_str(&row.get::<String, _>("decisions"))
        .map_err(|e| Error::Internal(format!("bad decisions JSON: {e}")))?;

    let artifacts: Vec<String> = serde_json::from_str(&row.get::<String, _>("artifacts"))
        .map_err(|e| Error::Internal(format!("bad artifacts JSON: {e}")))?;

    Ok(Session {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(SessionId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        task_id: row
            .get::<String, _>("task_id")
            .parse()
            .map(TaskId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        identity,
        started_at: parse_dt(&row.get::<String, _>("started_at"))?,
        ended_at: parse_dt(&row.get::<String, _>("ended_at"))?,
        outcome: row
            .get::<String, _>("outcome")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        failure_reason: row.get("failure_reason"),
        summary: row.get("summary"),
        decisions,
        knowledge_items: vec![], // Loaded separately when needed.
        artifacts,
        created_at: parse_dt(&row.get::<String, _>("created_at"))?,
    })
}

fn row_to_knowledge(row: &sqlx::sqlite::SqliteRow) -> Result<KnowledgeItem> {
    let task_id: Option<TaskId> = row
        .get::<Option<String>, _>("task_id")
        .map(|s| s.parse().map(TaskId::from_uuid))
        .transpose()
        .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;

    let session_id: Option<SessionId> = row
        .get::<Option<String>, _>("session_id")
        .map(|s| s.parse().map(SessionId::from_uuid))
        .transpose()
        .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?;

    Ok(KnowledgeItem {
        id: row
            .get::<String, _>("id")
            .parse()
            .map(KnowledgeId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        knowledge_type: row
            .get::<String, _>("type")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        title: row.get("title"),
        content: row.get("content"),
        scope: row
            .get::<String, _>("scope")
            .parse()
            .map_err(|e: String| Error::Internal(e))?,
        task_id,
        session_id,
        project_id: row
            .get::<String, _>("project_id")
            .parse()
            .map(ProjectId::from_uuid)
            .map_err(|e| Error::Internal(format!("bad UUID: {e}")))?,
        created_at: parse_dt(&row.get::<String, _>("created_at"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> Identity {
        Identity {
            harness: "claude-code".into(),
            agent_model: "opus-5".into(),
            session_id: "test-session".into(),
            label: None,
        }
    }

    #[tokio::test]
    async fn project_crud() {
        let store = Store::new_in_memory().await.unwrap();

        let p = store
            .create_project(&ProjectCreate {
                name: "test".into(),
                description: Some("desc".into()),
                settings: None,
            })
            .await
            .unwrap();

        assert_eq!(p.name, "test");
        assert!(p.settings.review_gate);

        let p2 = store.get_project(p.id).await.unwrap();
        assert_eq!(p2.name, "test");

        let p3 = store
            .update_project(
                p.id,
                &ProjectUpdate {
                    name: Some("updated".into()),
                    description: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(p3.name, "updated");

        store.delete_project(p.id).await.unwrap();
        assert!(store.get_project(p.id).await.is_err());
    }

    #[tokio::test]
    async fn task_lifecycle_happy_path() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        // Create task as proposed.
        let t = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task 1".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(t.status, TaskStatus::Proposed);

        // Approve → should auto-ready (no deps).
        let t = store.approve_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);

        // Claim.
        let now = Utc::now();
        let claim = store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();
        assert!(!claim.lease_id.is_empty());

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);

        // Report success → in_review (review gate on).
        let session = store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: Utc::now(),
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("done".into()),
                    decisions: Some(vec!["chose X".into()]),
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(session.outcome, SessionOutcome::Succeeded);

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InReview);
        assert_eq!(t.attempt_count, 1);
    }

    #[tokio::test]
    async fn dependency_cycle_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let t1 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "a".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let t2 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "b".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        // a depends on b.
        store
            .create_relation(
                p.id,
                t1.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t2.id,
                },
            )
            .await
            .unwrap();

        // b depends on a → cycle.
        let err = store
            .create_relation(
                p.id,
                t2.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t1.id,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:dependency-cycle");
    }

    #[tokio::test]
    async fn auto_ready_cascade_on_done() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: Some(ProjectSettings { review_gate: false }),
            })
            .await
            .unwrap();

        // Create two tasks: t2 depends on t1.
        let t1 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "dep".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let t2 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "main".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        // t2 depends_on t1.
        store
            .create_relation(
                p.id,
                t2.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t1.id,
                },
            )
            .await
            .unwrap();

        // t2 should be approved (not ready, since t1 is not done).
        let t2 = store.get_task(p.id, t2.id).await.unwrap();
        assert_eq!(t2.status, TaskStatus::Approved);

        // Complete t1: claim → report success (gate off → done).
        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t1.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        store
            .create_session(
                p.id,
                t1.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: Utc::now(),
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("done".into()),
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();

        // t2 should now be ready (auto-readied by cascade).
        let t2 = store.get_task(p.id, t2.id).await.unwrap();
        assert_eq!(t2.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn export_import_roundtrip() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "original".into(),
                description: Some("test export".into()),
                settings: None,
            })
            .await
            .unwrap();

        store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let doc = store.export_project(p.id).await.unwrap();
        assert_eq!(doc.tasks.len(), 1);

        let result = store.import_project(&doc).await.unwrap();
        assert_ne!(result.project_id, p.id); // New IDs.
        assert_eq!(result.task_count, 1);

        // Verify imported project exists.
        let imported = store.get_project(result.project_id).await.unwrap();
        assert_eq!(imported.name, "original");
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn other_identity() -> Identity {
        Identity {
            harness: "cursor".into(),
            agent_model: "sonnet-5".into(),
            session_id: "other-session".into(),
            label: None,
        }
    }

    /// Create a project with review_gate off for quick done transitions.
    async fn project_no_gate(store: &Store) -> Project {
        store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: Some(ProjectSettings { review_gate: false }),
            })
            .await
            .unwrap()
    }

    /// Create a proposed task.
    async fn proposed_task(store: &Store, pid: ProjectId, title: &str) -> Task {
        store
            .create_task(
                pid,
                &TaskCreate {
                    title: title.into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap()
    }

    /// Create an approved (auto-readied) task.
    async fn ready_task(store: &Store, pid: ProjectId, title: &str) -> Task {
        let t = proposed_task(store, pid, title).await;
        store.approve_task(pid, t.id).await.unwrap()
    }

    /// Drive a task all the way to in_progress.
    async fn in_progress_task(store: &Store, pid: ProjectId, title: &str) -> (Task, Claim) {
        let t = ready_task(store, pid, title).await;
        let now = Utc::now();
        let claim = store
            .claim_task(
                pid,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();
        let t = store.get_task(pid, t.id).await.unwrap();
        (t, claim)
    }

    /// Drive a task all the way to done (needs review_gate off).
    async fn done_task(store: &Store, pid: ProjectId, title: &str) -> Task {
        let (t, _claim) = in_progress_task(store, pid, title).await;
        let now = Utc::now();
        store
            .create_session(
                pid,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("done".into()),
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();
        store.get_task(pid, t.id).await.unwrap()
    }

    // ── update_task ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn update_task_fields() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "original").await;

        let updated = store
            .update_task(
                p.id,
                t.id,
                &TaskUpdate {
                    title: Some("renamed".into()),
                    description: Some("new desc".into()),
                    task_type: Some(TaskType::Research),
                    metadata: Some(serde_json::json!({"key": "val"})),
                    assignee: Some(Some(test_identity())),
                    graph_role: Some(vec![GraphRole::Start]),
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.title, "renamed");
        assert_eq!(updated.description, "new desc");
        assert_eq!(updated.task_type, TaskType::Research);
        assert_eq!(updated.metadata["key"], "val");
        assert!(updated.assignee.is_some());
        assert!(updated.graph_role.contains(&GraphRole::Start));
    }

    #[tokio::test]
    async fn update_task_clear_assignee() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "t".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: None,
                    assignee: Some(test_identity()),
                    graph_role: None,
                },
            )
            .await
            .unwrap();
        assert!(t.assignee.is_some());

        let updated = store
            .update_task(
                p.id,
                t.id,
                &TaskUpdate {
                    title: None,
                    description: None,
                    task_type: None,
                    metadata: None,
                    assignee: Some(None), // explicit clear
                    graph_role: None,
                },
            )
            .await
            .unwrap();
        assert!(updated.assignee.is_none());
    }

    #[tokio::test]
    async fn update_task_invalid_metadata_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "t").await;

        let mut map = serde_json::Map::new();
        for i in 0..=200 {
            map.insert(format!("k{i}"), serde_json::Value::Null);
        }
        let err = store
            .update_task(
                p.id,
                t.id,
                &TaskUpdate {
                    title: None,
                    description: None,
                    task_type: None,
                    metadata: Some(serde_json::Value::Object(map)),
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:validation-error");
    }

    // ── delete_task ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_task_soft_deletes() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "deletable").await;

        store.delete_task(p.id, t.id).await.unwrap();

        // get_task should return not-found.
        let err = store.get_task(p.id, t.id).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:not-found");

        // list_tasks should not include it.
        let page = store.list_tasks(p.id, None, 25, None, None).await.unwrap();
        assert!(page.items.is_empty());
    }

    // ── list_projects ───────────────────────────────────────────────────

    #[tokio::test]
    async fn list_projects_empty() {
        let store = Store::new_in_memory().await.unwrap();
        let page = store.list_projects(None, 25).await.unwrap();
        assert!(page.items.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn list_projects_pagination() {
        let store = Store::new_in_memory().await.unwrap();

        for i in 0..5 {
            store
                .create_project(&ProjectCreate {
                    name: format!("p{i}"),
                    description: None,
                    settings: None,
                })
                .await
                .unwrap();
        }

        let page1 = store.list_projects(None, 3).await.unwrap();
        assert_eq!(page1.items.len(), 3);
        assert!(page1.has_more);
        assert!(page1.next_cursor.is_some());

        let page2 = store
            .list_projects(page1.next_cursor.as_deref(), 3)
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 2);
        assert!(!page2.has_more);
    }

    // ── list_tasks ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_tasks_with_status_filter() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        proposed_task(&store, p.id, "proposed1").await;
        proposed_task(&store, p.id, "proposed2").await;
        ready_task(&store, p.id, "ready1").await;

        let proposed = store
            .list_tasks(p.id, None, 25, Some(TaskStatus::Proposed), None)
            .await
            .unwrap();
        assert_eq!(proposed.items.len(), 2);

        let ready = store
            .list_tasks(p.id, None, 25, Some(TaskStatus::Ready), None)
            .await
            .unwrap();
        assert_eq!(ready.items.len(), 1);
    }

    #[tokio::test]
    async fn list_tasks_with_type_filter() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        proposed_task(&store, p.id, "code-task").await;
        store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "research-task".into(),
                    description: None,
                    task_type: TaskType::Research,
                    status: None,
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let code_tasks = store
            .list_tasks(p.id, None, 25, None, Some(TaskType::Code))
            .await
            .unwrap();
        assert_eq!(code_tasks.items.len(), 1);

        let research_tasks = store
            .list_tasks(p.id, None, 25, None, Some(TaskType::Research))
            .await
            .unwrap();
        assert_eq!(research_tasks.items.len(), 1);
    }

    #[tokio::test]
    async fn list_tasks_pagination() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        for i in 0..5 {
            proposed_task(&store, p.id, &format!("t{i}")).await;
        }

        let page1 = store.list_tasks(p.id, None, 2, None, None).await.unwrap();
        assert_eq!(page1.items.len(), 2);
        assert!(page1.has_more);

        let page2 = store
            .list_tasks(p.id, page1.next_cursor.as_deref(), 2, None, None)
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 2);
        assert!(page2.has_more);

        let page3 = store
            .list_tasks(p.id, page2.next_cursor.as_deref(), 2, None, None)
            .await
            .unwrap();
        assert_eq!(page3.items.len(), 1);
        assert!(!page3.has_more);
    }

    // ── next_task ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn next_task_returns_none_when_empty() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        assert!(store.next_task(p.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn next_task_returns_only_ready_unclaimed() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // proposed task should not be returned
        proposed_task(&store, p.id, "not-ready").await;
        assert!(store.next_task(p.id).await.unwrap().is_none());

        // ready task should be returned
        let t = ready_task(&store, p.id, "ready").await;
        let next = store.next_task(p.id).await.unwrap().unwrap();
        assert_eq!(next.id, t.id);
    }

    #[tokio::test]
    async fn next_task_graph_aware_priority() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // Create: t_leaf (no dependents), t_root (2 downstream dependents).
        // t_root is a prerequisite for t_mid, t_mid is prerequisite for t_leaf.
        // So completing t_root unblocks more work.
        let t_root = ready_task(&store, p.id, "root").await;
        let t_mid = proposed_task(&store, p.id, "mid").await;
        let t_leaf = proposed_task(&store, p.id, "leaf").await;

        // t_mid depends_on t_root
        store
            .create_relation(
                p.id,
                t_mid.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t_root.id,
                },
            )
            .await
            .unwrap();

        // t_leaf depends_on t_mid
        store
            .create_relation(
                p.id,
                t_leaf.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t_mid.id,
                },
            )
            .await
            .unwrap();

        // Also create an isolated ready task with no downstream.
        let t_isolated = ready_task(&store, p.id, "isolated").await;

        // next_task should prefer t_root (downstream count = 2) over t_isolated (0).
        let next = store.next_task(p.id).await.unwrap().unwrap();
        assert_eq!(
            next.id, t_root.id,
            "should pick task with most downstream work"
        );

        // After claiming t_root, next_task should return t_isolated.
        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t_root.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        let next = store.next_task(p.id).await.unwrap().unwrap();
        assert_eq!(next.id, t_isolated.id);
    }

    // ── reject_task ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn reject_task_returns_to_ready() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: None, // review_gate = true
            })
            .await
            .unwrap();

        let (t, _) = in_progress_task(&store, p.id, "t").await;
        let now = Utc::now();

        // Report success → in_review (gate on).
        store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("done".into()),
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InReview);

        // Reject → ready.
        let t = store.reject_task(p.id, t.id, "needs work").await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
    }

    // ── block / unblock ─────────────────────────────────────────────────

    #[tokio::test]
    async fn block_and_unblock_restores_status() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "blockable").await;
        assert_eq!(t.status, TaskStatus::Ready);

        let blocked = store
            .block_task(p.id, t.id, "waiting on design")
            .await
            .unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        assert_eq!(blocked.blocked_from_status, Some(TaskStatus::Ready));
        assert_eq!(blocked.block_reason.as_deref(), Some("waiting on design"));

        let unblocked = store.unblock_task(p.id, t.id).await.unwrap();
        assert_eq!(unblocked.status, TaskStatus::Ready);
        assert!(unblocked.blocked_from_status.is_none());
        assert!(unblocked.block_reason.is_none());
    }

    #[tokio::test]
    async fn block_terminal_task_fails() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = done_task(&store, p.id, "finished").await;

        let err = store.block_task(p.id, t.id, "nope").await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:invalid-transition");
    }

    // ── cancel_task ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_task_from_various_states() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // Cancel from proposed.
        let t1 = proposed_task(&store, p.id, "cancel-proposed").await;
        let c = store.cancel_task(p.id, t1.id).await.unwrap();
        assert_eq!(c.status, TaskStatus::Cancelled);

        // Cancel from ready.
        let t2 = ready_task(&store, p.id, "cancel-ready").await;
        let c = store.cancel_task(p.id, t2.id).await.unwrap();
        assert_eq!(c.status, TaskStatus::Cancelled);

        // Cancel from blocked.
        let t3 = ready_task(&store, p.id, "cancel-blocked").await;
        store.block_task(p.id, t3.id, "reason").await.unwrap();
        let c = store.cancel_task(p.id, t3.id).await.unwrap();
        assert_eq!(c.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_done_task_fails() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = done_task(&store, p.id, "done").await;

        let err = store.cancel_task(p.id, t.id).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:invalid-transition");
    }

    // ── delete_relation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_relation_triggers_auto_ready() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let t1 = proposed_task(&store, p.id, "dep").await;
        let t2 = proposed_task(&store, p.id, "main").await;

        // Approve both.
        store.approve_task(p.id, t1.id).await.unwrap();
        store.approve_task(p.id, t2.id).await.unwrap();

        // t2 depends_on t1 → t2 demoted to approved.
        let rel = store
            .create_relation(
                p.id,
                t2.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t1.id,
                },
            )
            .await
            .unwrap();

        let t2 = store.get_task(p.id, t2.id).await.unwrap();
        assert_eq!(t2.status, TaskStatus::Approved);

        // Delete the dependency → t2 should auto-ready.
        store.delete_relation(p.id, t2.id, rel.id).await.unwrap();

        let t2 = store.get_task(p.id, t2.id).await.unwrap();
        assert_eq!(t2.status, TaskStatus::Ready);
    }

    // ── list_relations ──────────────────────────────────────────────────

    #[tokio::test]
    async fn list_relations_for_task() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let t1 = proposed_task(&store, p.id, "a").await;
        let t2 = proposed_task(&store, p.id, "b").await;
        let t3 = proposed_task(&store, p.id, "c").await;

        store
            .create_relation(
                p.id,
                t1.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t2.id,
                },
            )
            .await
            .unwrap();
        store
            .create_relation(
                p.id,
                t3.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t1.id,
                },
            )
            .await
            .unwrap();

        // t1 is involved in both relations (as source of one, target of another).
        let rels = store.list_relations(p.id, t1.id).await.unwrap();
        assert_eq!(rels.len(), 2);
    }

    // ── renew_claim ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn renew_claim_extends_ttl() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, claim) = in_progress_task(&store, p.id, "renewable").await;

        let now = Utc::now();
        let renewed = store
            .renew_claim(
                p.id,
                t.id,
                &ClaimRenewal {
                    identity: test_identity(),
                    ttl_seconds: Some(600),
                },
                now,
            )
            .await
            .unwrap();

        assert_eq!(renewed.id, claim.id);
        assert_eq!(renewed.ttl_seconds, 600);
        assert!(renewed.renewed_at.is_some());
        assert!(renewed.expires_at > claim.expires_at);
    }

    #[tokio::test]
    async fn renew_claim_identity_mismatch_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "t").await;

        let now = Utc::now();
        let err = store
            .renew_claim(
                p.id,
                t.id,
                &ClaimRenewal {
                    identity: other_identity(),
                    ttl_seconds: None,
                },
                now,
            )
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:claim-conflict");
    }

    #[tokio::test]
    async fn renew_claim_default_ttl() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, claim) = in_progress_task(&store, p.id, "t").await;

        let now = Utc::now();
        let renewed = store
            .renew_claim(
                p.id,
                t.id,
                &ClaimRenewal {
                    identity: test_identity(),
                    ttl_seconds: None, // should reuse original TTL
                },
                now,
            )
            .await
            .unwrap();

        assert_eq!(renewed.ttl_seconds, claim.ttl_seconds);
    }

    // ── release_claim ───────────────────────────────────────────────────

    #[tokio::test]
    async fn release_claim_returns_task_to_ready() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "releasable").await;

        let now = Utc::now();
        store
            .release_claim(
                p.id,
                t.id,
                &ClaimRelease {
                    identity: test_identity(),
                },
                now,
            )
            .await
            .unwrap();

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn release_claim_identity_mismatch_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "t").await;

        let now = Utc::now();
        let err = store
            .release_claim(
                p.id,
                t.id,
                &ClaimRelease {
                    identity: other_identity(),
                },
                now,
            )
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:claim-conflict");
    }

    // ── sweep_expired_claims ────────────────────────────────────────────

    #[tokio::test]
    async fn sweep_expired_claims_releases_and_returns_to_ready() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "expirable").await;

        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 60,
                },
                now,
            )
            .await
            .unwrap();

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);

        // Advance time past expiry.
        let future = now + chrono::TimeDelta::seconds(120);
        let released = store.sweep_expired_claims(future).await.unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0], t.id);

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn sweep_no_expired_claims_returns_empty() {
        let store = Store::new_in_memory().await.unwrap();
        let released = store.sweep_expired_claims(Utc::now()).await.unwrap();
        assert!(released.is_empty());
    }

    // ── sessions ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn failed_session_returns_task_to_ready() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "fail-me").await;

        let now = Utc::now();
        let session = store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Failed,
                    failure_reason: Some("timeout".into()),
                    summary: Some("failed attempt".into()),
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(session.outcome, SessionOutcome::Failed);
        assert_eq!(session.failure_reason.as_deref(), Some("timeout"));

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
        assert_eq!(t.attempt_count, 1);
    }

    #[tokio::test]
    async fn list_sessions_returns_task_sessions() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "multi-session").await;

        let now = Utc::now();
        // First session: fail
        store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Failed,
                    failure_reason: Some("oops".into()),
                    summary: None,
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();

        // Task back to ready → claim again → second session.
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);

        let now2 = Utc::now();
        store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now2,
            )
            .await
            .unwrap();

        store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now2,
                    ended_at: now2,
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("success".into()),
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap();

        let page = store.list_sessions(p.id, t.id, None, 25).await.unwrap();
        assert_eq!(page.items.len(), 2);

        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.attempt_count, 2);
    }

    #[tokio::test]
    async fn session_on_non_in_progress_task_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "not-in-progress").await;

        let now = Utc::now();
        let err = store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: None,
                    decisions: None,
                    knowledge_items: None,
                    artifacts: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:invalid-transition");
    }

    #[tokio::test]
    async fn session_with_inline_knowledge_items() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _) = in_progress_task(&store, p.id, "with-ki").await;

        let now = Utc::now();
        store
            .create_session(
                p.id,
                t.id,
                &SessionReport {
                    identity: test_identity(),
                    started_at: now,
                    ended_at: now,
                    outcome: SessionOutcome::Succeeded,
                    failure_reason: None,
                    summary: Some("done".into()),
                    decisions: Some(vec!["chose rust".into()]),
                    knowledge_items: Some(vec![KnowledgeItemCreate {
                        knowledge_type: KnowledgeType::Decision,
                        title: "picked rust".into(),
                        content: "Rust is the best choice".into(),
                        scope: KnowledgeScope::Task,
                        task_id: None,
                        session_id: None,
                    }]),
                    artifacts: Some(vec!["https://github.com/pr/1".into()]),
                },
            )
            .await
            .unwrap();

        // Knowledge item should exist.
        let page = store
            .list_knowledge(p.id, None, 25, None, None, None)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title, "picked rust");
    }

    // ── knowledge CRUD ──────────────────────────────────────────────────

    #[tokio::test]
    async fn knowledge_crud() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let ki = store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Note,
                    title: "convention".into(),
                    content: "use snake_case".into(),
                    scope: KnowledgeScope::Project,
                    task_id: None,
                    session_id: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(ki.title, "convention");
        assert_eq!(ki.scope, KnowledgeScope::Project);

        let fetched = store.get_knowledge(p.id, ki.id).await.unwrap();
        assert_eq!(fetched.content, "use snake_case");

        store.delete_knowledge(p.id, ki.id).await.unwrap();
        assert!(store.get_knowledge(p.id, ki.id).await.is_err());
    }

    #[tokio::test]
    async fn list_knowledge_filters() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "t").await;

        // Project-scoped note.
        store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Note,
                    title: "global".into(),
                    content: "global knowledge".into(),
                    scope: KnowledgeScope::Project,
                    task_id: None,
                    session_id: None,
                },
            )
            .await
            .unwrap();

        // Task-scoped decision.
        store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Decision,
                    title: "task-decision".into(),
                    content: "decided X".into(),
                    scope: KnowledgeScope::Task,
                    task_id: Some(t.id),
                    session_id: None,
                },
            )
            .await
            .unwrap();

        // Filter by scope.
        let project_ki = store
            .list_knowledge(p.id, None, 25, Some(KnowledgeScope::Project), None, None)
            .await
            .unwrap();
        assert_eq!(project_ki.items.len(), 1);

        // Filter by type.
        let decisions = store
            .list_knowledge(p.id, None, 25, None, Some(KnowledgeType::Decision), None)
            .await
            .unwrap();
        assert_eq!(decisions.items.len(), 1);

        // Filter by task_id.
        let task_ki = store
            .list_knowledge(p.id, None, 25, None, None, Some(t.id))
            .await
            .unwrap();
        assert_eq!(task_ki.items.len(), 1);

        // No filter returns all.
        let all = store
            .list_knowledge(p.id, None, 25, None, None, None)
            .await
            .unwrap();
        assert_eq!(all.items.len(), 2);
    }

    // ── context_bundle ──────────────────────────────────────────────────

    #[tokio::test]
    async fn context_bundle_assembles_ancestors_and_knowledge() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // Add project knowledge.
        store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Note,
                    title: "global".into(),
                    content: "project convention".into(),
                    scope: KnowledgeScope::Project,
                    task_id: None,
                    session_id: None,
                },
            )
            .await
            .unwrap();

        // Create chain: t_dep (done) → t_main (depends on t_dep).
        let t_dep = done_task(&store, p.id, "dep-task").await;

        let t_main = proposed_task(&store, p.id, "main-task").await;
        store.approve_task(p.id, t_main.id).await.unwrap();

        store
            .create_relation(
                p.id,
                t_main.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t_dep.id,
                },
            )
            .await
            .unwrap();

        let bundle = store.get_context_bundle(p.id, t_main.id).await.unwrap();

        assert_eq!(bundle.task.id, t_main.id);
        assert_eq!(bundle.ancestor_summaries.len(), 1);
        assert_eq!(bundle.ancestor_summaries[0].task_id, t_dep.id);
        assert_eq!(bundle.project_knowledge.len(), 1);
    }

    #[tokio::test]
    async fn context_bundle_includes_siblings() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // Two tasks in progress at the same time.
        let (t1, _) = in_progress_task(&store, p.id, "sibling-a").await;
        let t2 = ready_task(&store, p.id, "sibling-b").await;

        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t2.id,
                &ClaimRequest {
                    identity: other_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        let bundle = store.get_context_bundle(p.id, t1.id).await.unwrap();

        // t2 should be in siblings (in_progress, claimed by other_identity).
        assert!(
            bundle.sibling_tasks.iter().any(|s| s.task_id == t2.id),
            "should include sibling task"
        );
    }

    // ── decomposition single-parent ─────────────────────────────────────

    #[tokio::test]
    async fn decomposition_single_parent_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let parent1 = proposed_task(&store, p.id, "parent1").await;
        let parent2 = proposed_task(&store, p.id, "parent2").await;
        let child = proposed_task(&store, p.id, "child").await;

        // First parent → child: ok.
        store
            .create_relation(
                p.id,
                parent1.id,
                &RelationCreate {
                    relation_type: RelationType::Decomposition,
                    target_task_id: child.id,
                },
            )
            .await
            .unwrap();

        // Second parent → same child: rejected.
        let err = store
            .create_relation(
                p.id,
                parent2.id,
                &RelationCreate {
                    relation_type: RelationType::Decomposition,
                    target_task_id: child.id,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:decomposition-violation");
    }

    // ── ready demotion ──────────────────────────────────────────────────

    #[tokio::test]
    async fn ready_demotion_on_new_undone_dependency() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let t_ready = ready_task(&store, p.id, "was-ready").await;
        assert_eq!(t_ready.status, TaskStatus::Ready);

        let t_dep = proposed_task(&store, p.id, "undone-dep").await;

        // Add dependency: t_ready depends_on t_dep (which is not done).
        store
            .create_relation(
                p.id,
                t_ready.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t_dep.id,
                },
            )
            .await
            .unwrap();

        let t_ready = store.get_task(p.id, t_ready.id).await.unwrap();
        assert_eq!(
            t_ready.status,
            TaskStatus::Approved,
            "should be demoted to approved"
        );
    }

    // ── task creation edge cases ────────────────────────────────────────

    #[tokio::test]
    async fn task_created_as_approved_auto_readies() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let t = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "auto-ready".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        // No deps → auto-readied.
        assert_eq!(t.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn task_invalid_initial_status_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let err = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "bad-status".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Done),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:validation-error");
    }

    #[tokio::test]
    async fn task_invalid_metadata_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let mut map = serde_json::Map::new();
        for i in 0..=200 {
            map.insert(format!("k{i}"), serde_json::Value::Null);
        }

        let err = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "too-much-meta".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: None,
                    metadata: Some(serde_json::Value::Object(map)),
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:validation-error");
    }

    // ── claim edge cases ────────────────────────────────────────────────

    #[tokio::test]
    async fn claim_non_ready_task_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "not-ready").await;

        let now = Utc::now();
        let err = store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:task-not-ready");
    }

    #[tokio::test]
    async fn claim_already_claimed_task_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "single-claim").await;

        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        // Can't claim again — someone else holds the lease.
        let err = store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: other_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap_err();

        assert_eq!(err.urn(), "urn:shepherd:error:claim-conflict");
    }

    // ── self-relation rejected ──────────────────────────────────────────

    #[tokio::test]
    async fn self_relation_rejected() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = proposed_task(&store, p.id, "self").await;

        let err = store
            .create_relation(
                p.id,
                t.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t.id,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:validation-error");
    }

    // ── export with full data ───────────────────────────────────────────

    #[tokio::test]
    async fn export_import_with_relations_sessions_knowledge() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        let t1 = proposed_task(&store, p.id, "t1").await;
        let t2 = proposed_task(&store, p.id, "t2").await;

        // Add a relation.
        store
            .create_relation(
                p.id,
                t1.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: t2.id,
                },
            )
            .await
            .unwrap();

        // Add knowledge.
        store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Note,
                    title: "note".into(),
                    content: "content".into(),
                    scope: KnowledgeScope::Project,
                    task_id: None,
                    session_id: None,
                },
            )
            .await
            .unwrap();

        let doc = store.export_project(p.id).await.unwrap();
        assert_eq!(doc.tasks.len(), 2);
        assert_eq!(doc.relations.len(), 1);
        assert_eq!(doc.knowledge_items.len(), 1);

        let result = store.import_project(&doc).await.unwrap();
        assert_eq!(result.task_count, 2);
        assert_eq!(result.relation_count, 1);
        assert_eq!(result.knowledge_count, 1);
    }

    // ── not-found errors ────────────────────────────────────────────────

    #[tokio::test]
    async fn get_nonexistent_project_returns_not_found() {
        let store = Store::new_in_memory().await.unwrap();
        let err = store.get_project(ProjectId::new()).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:not-found");
    }

    #[tokio::test]
    async fn get_nonexistent_task_returns_not_found() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let err = store.get_task(p.id, TaskId::new()).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:not-found");
    }

    #[tokio::test]
    async fn get_nonexistent_knowledge_returns_not_found() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let err = store
            .get_knowledge(p.id, KnowledgeId::new())
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:not-found");
    }

    // ── unblock non-blocked task ────────────────────────────────────────

    #[tokio::test]
    async fn unblock_non_blocked_task_fails() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "not-blocked").await;

        let err = store.unblock_task(p.id, t.id).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:invalid-transition");
    }

    // ── project update with settings ────────────────────────────────────

    #[tokio::test]
    async fn update_project_review_gate() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "gated".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();
        assert!(p.settings.review_gate);

        let updated = store
            .update_project(
                p.id,
                &ProjectUpdate {
                    name: None,
                    description: None,
                    settings: Some(ProjectSettings { review_gate: false }),
                },
            )
            .await
            .unwrap();
        assert!(!updated.settings.review_gate);
    }

    // ── lease expiry through lazy check ─────────────────────────────────

    #[tokio::test]
    async fn expired_claim_released_lazily_on_next_claim() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "lazy-expiry").await;

        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 60,
                },
                now,
            )
            .await
            .unwrap();

        // Renew with expired time → should get LeaseExpired (lazy expiry
        // releases the claim when it checks).
        let future = now + chrono::TimeDelta::seconds(120);
        let err = store
            .renew_claim(
                p.id,
                t.id,
                &ClaimRenewal {
                    identity: test_identity(),
                    ttl_seconds: None,
                },
                future,
            )
            .await
            .unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:lease-expired");
    }

    // ── approve already-approved task ───────────────────────────────────

    #[tokio::test]
    async fn approve_non_proposed_task_fails() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "already-ready").await;

        let err = store.approve_task(p.id, t.id).await.unwrap_err();
        assert_eq!(err.urn(), "urn:shepherd:error:invalid-transition");
    }

    // ── delete_project hides tasks in list ──────────────────────────────

    #[tokio::test]
    async fn deleted_project_not_in_list() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "deletable".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let page = store.list_projects(None, 25).await.unwrap();
        assert_eq!(page.items.len(), 1);

        store.delete_project(p.id).await.unwrap();

        let page = store.list_projects(None, 25).await.unwrap();
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn delete_project_cascades_to_tasks() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: Some(ProjectSettings { review_gate: false }),
            })
            .await
            .unwrap();

        let t1 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task1".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let t2 = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task2".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        // Claim t1 so it has an active claim.
        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t1.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        // Delete project — should cascade to both tasks.
        store.delete_project(p.id).await.unwrap();

        // Both tasks should be invisible.
        assert!(store.get_task(p.id, t1.id).await.is_err());
        assert!(store.get_task(p.id, t2.id).await.is_err());

        // The claim on t1 should have been released.
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM claims WHERE task_id = ? AND released_at IS NULL",
        )
        .bind(t1.id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(active, 0, "claim should be released on cascade delete");
    }

    #[tokio::test]
    async fn delete_task_releases_active_claim() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let t = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let now = Utc::now();
        store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();

        store.delete_task(p.id, t.id).await.unwrap();

        // Claim should be released with reason "task_deleted".
        let reason: Option<String> =
            sqlx::query_scalar("SELECT release_reason FROM claims WHERE task_id = ?")
                .bind(t.id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(reason, Some("task_deleted".into()));
    }

    #[tokio::test]
    async fn delete_task_auto_readies_dependents() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "proj".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let dep = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "dep".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        let main_task = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "main".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        // main depends_on dep.
        store
            .create_relation(
                p.id,
                main_task.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: dep.id,
                },
            )
            .await
            .unwrap();

        // main should be approved (dep not done).
        let main_task = store.get_task(p.id, main_task.id).await.unwrap();
        assert_eq!(main_task.status, TaskStatus::Approved);

        // Delete the dependency — main should auto-ready.
        store.delete_task(p.id, dep.id).await.unwrap();

        let main_task = store.get_task(p.id, main_task.id).await.unwrap();
        assert_eq!(main_task.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn purge_project_hard_deletes_everything() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "to-purge".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let t = store
            .create_task(
                p.id,
                &TaskCreate {
                    title: "task".into(),
                    description: None,
                    task_type: TaskType::Code,
                    status: Some(TaskStatus::Approved),
                    metadata: None,
                    assignee: None,
                    graph_role: None,
                },
            )
            .await
            .unwrap();

        store
            .create_knowledge(
                p.id,
                &KnowledgeItemCreate {
                    knowledge_type: KnowledgeType::Note,
                    title: "note".into(),
                    content: "content".into(),
                    scope: KnowledgeScope::Project,
                    task_id: None,
                    session_id: None,
                },
            )
            .await
            .unwrap();

        // Soft-delete first (required for purge).
        store.delete_project(p.id).await.unwrap();

        // Purge.
        store.purge_project(p.id).await.unwrap();

        // Everything should be physically gone.
        let projects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE id = ?")
            .bind(p.id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(projects, 0, "project row should be physically deleted");

        let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE id = ?")
            .bind(t.id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(tasks, 0, "task row should be physically deleted");

        let knowledge: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items WHERE project_id = ?")
                .bind(p.id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(knowledge, 0, "knowledge rows should be physically deleted");
    }

    #[tokio::test]
    async fn purge_project_rejects_non_deleted() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "active".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        let err = store.purge_project(p.id).await.unwrap_err();
        assert_eq!(err.status_code(), 422);
    }

    #[tokio::test]
    async fn purge_all_deleted_respects_threshold() {
        let store = Store::new_in_memory().await.unwrap();
        let p = store
            .create_project(&ProjectCreate {
                name: "old".into(),
                description: None,
                settings: None,
            })
            .await
            .unwrap();

        store.delete_project(p.id).await.unwrap();

        // Purge with a future threshold — should catch it.
        let future = Utc::now() + chrono::TimeDelta::hours(1);
        let result = store.purge_all_deleted(future).await.unwrap();
        assert_eq!(result.projects_purged, 1);

        // Nothing left to purge.
        let result2 = store.purge_all_deleted(future).await.unwrap();
        assert_eq!(result2.projects_purged, 0);
    }

    // ── S5: claim guard, expiry, import normalization ────────────────────

    /// Force a task's active claim to look expired (simulates a crashed agent).
    async fn force_expire_claim(store: &Store, task_id: TaskId) {
        let past = (Utc::now() - chrono::TimeDelta::hours(1)).to_rfc3339();
        sqlx::query("UPDATE claims SET expires_at = ? WHERE task_id = ? AND released_at IS NULL")
            .bind(past)
            .bind(task_id.to_string())
            .execute(store.pool())
            .await
            .unwrap();
    }

    fn session_report(identity: Identity, outcome: SessionOutcome) -> SessionReport {
        let now = Utc::now();
        SessionReport {
            identity,
            started_at: now,
            ended_at: now,
            outcome,
            failure_reason: None,
            summary: Some("report".into()),
            decisions: None,
            knowledge_items: None,
            artifacts: None,
        }
    }

    #[tokio::test]
    async fn session_report_rejects_identity_mismatch() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, claim) = in_progress_task(&store, p.id, "guarded").await;

        let err = store
            .create_session(
                p.id,
                t.id,
                &session_report(other_identity(), SessionOutcome::Succeeded),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), 409);
        assert_eq!(err.urn(), "urn:shepherd:error:claim-conflict");

        // The rightful claim is untouched and the task still in progress.
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
        let active = store.get_active_claim(t.id, Utc::now()).await.unwrap();
        assert_eq!(active.unwrap().id, claim.id);
    }

    #[tokio::test]
    async fn session_report_rejects_expired_claim() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _claim) = in_progress_task(&store, p.id, "expired").await;
        force_expire_claim(&store, t.id).await;

        let err = store
            .create_session(
                p.id,
                t.id,
                &session_report(test_identity(), SessionOutcome::Succeeded),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), 410);
        assert_eq!(err.urn(), "urn:shepherd:error:lease-expired");
    }

    #[tokio::test]
    async fn next_task_sweeps_expired_leases() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _claim) = in_progress_task(&store, p.id, "crashed").await;
        force_expire_claim(&store, t.id).await;

        // The lazy sweep returns the task to ready and offers it again.
        let next = store.next_task(p.id).await.unwrap().unwrap();
        assert_eq!(next.id, t.id);
        assert_eq!(next.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn claim_succeeds_over_expired_unswept_claim() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _claim) = in_progress_task(&store, p.id, "reclaim").await;
        force_expire_claim(&store, t.id).await;
        // Sweep returns the task to ready.
        store.sweep_expired_claims(Utc::now()).await.unwrap();

        let claim2 = store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: other_identity(),
                    ttl_seconds: 300,
                },
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(claim2.identity.matches(&other_identity()));
    }

    #[tokio::test]
    async fn stale_claimant_cannot_hijack_reclaimed_task() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        // Agent A claims, then its lease expires.
        let (t, _claim_a) = in_progress_task(&store, p.id, "hijack").await;
        force_expire_claim(&store, t.id).await;
        store.sweep_expired_claims(Utc::now()).await.unwrap();

        // Agent B re-claims.
        let claim_b = store
            .claim_task(
                p.id,
                t.id,
                &ClaimRequest {
                    identity: other_identity(),
                    ttl_seconds: 300,
                },
                Utc::now(),
            )
            .await
            .unwrap();

        // A's late report must not release B's claim or advance the task.
        let err = store
            .create_session(
                p.id,
                t.id,
                &session_report(test_identity(), SessionOutcome::Succeeded),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), 409);

        let active = store.get_active_claim(t.id, Utc::now()).await.unwrap();
        assert_eq!(active.unwrap().id, claim_b.id);
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn import_normalizes_in_progress_tasks() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // a: done. b: in_progress, depends on a (deps satisfied).
        let a = done_task(&store, p.id, "a done").await;
        let (b, _claim) = in_progress_task(&store, p.id, "b in progress").await;
        store
            .create_relation(
                p.id,
                b.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: a.id,
                },
            )
            .await
            .unwrap();
        // c: in_progress with an unfinished dep (added while claimed).
        let (c, _claim) = in_progress_task(&store, p.id, "c in progress").await;
        let d = ready_task(&store, p.id, "d not done").await;
        store
            .create_relation(
                p.id,
                c.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: d.id,
                },
            )
            .await
            .unwrap();

        let doc = store.export_project(p.id).await.unwrap();
        let result = store.import_project(&doc).await.unwrap();

        let imported = store
            .list_tasks(result.project_id, None, 100, None, None)
            .await
            .unwrap();
        let status_of = |title: &str| {
            imported
                .items
                .iter()
                .find(|t| t.title == title)
                .unwrap()
                .status
        };
        assert_eq!(status_of("a done"), TaskStatus::Done);
        assert_eq!(status_of("b in progress"), TaskStatus::Ready);
        assert_eq!(status_of("c in progress"), TaskStatus::Approved);
        assert_eq!(status_of("d not done"), TaskStatus::Ready);
    }

    #[tokio::test]
    async fn sweep_rescues_stale_orphaned_in_progress() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, claim) = in_progress_task(&store, p.id, "orphaned").await;

        // Simulate a crash between claim release and the status update: the
        // row is released (beyond the 30s grace) but the task stays
        // in_progress.
        let stale = (Utc::now() - chrono::TimeDelta::seconds(60)).to_rfc3339();
        sqlx::query("UPDATE claims SET released_at = ?, release_reason = 'voluntary' WHERE id = ?")
            .bind(&stale)
            .bind(claim.id.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let released = store.sweep_expired_claims(Utc::now()).await.unwrap();
        assert!(released.contains(&t.id));
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn sweep_leaves_fresh_releases_alone() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, claim) = in_progress_task(&store, p.id, "mid-release").await;

        // A release within the grace window (a healthy release→status update
        // in flight) must not be rescued.
        sqlx::query("UPDATE claims SET released_at = ?, release_reason = 'voluntary' WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(claim.id.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let released = store.sweep_expired_claims(Utc::now()).await.unwrap();
        assert!(!released.contains(&t.id));
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn import_falls_back_blocked_from_status() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let t = ready_task(&store, p.id, "blocked export").await;
        store.block_task(p.id, t.id, "waiting").await.unwrap();

        // Simulate the wire round-trip: the spec's Task schema does not
        // carry blocked_from_status, so REST imports arrive without it.
        let mut doc = store.export_project(p.id).await.unwrap();
        for task in &mut doc.tasks {
            task.blocked_from_status = None;
        }

        let result = store.import_project(&doc).await.unwrap();
        let imported = store
            .list_tasks(result.project_id, None, 100, None, None)
            .await
            .unwrap();
        let blocked = &imported.items[0];
        assert_eq!(blocked.status, TaskStatus::Blocked);

        // The fallback keeps unblock working; deps are re-checked on the way.
        let unblocked = store
            .unblock_task(result.project_id, blocked.id)
            .await
            .unwrap();
        assert_eq!(unblocked.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn import_normalizes_approved_with_done_deps() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let a = done_task(&store, p.id, "dep done").await;
        let (b, _claim) = in_progress_task(&store, p.id, "was approved").await;
        store
            .create_relation(
                p.id,
                b.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: a.id,
                },
            )
            .await
            .unwrap();

        // Hand-crafted docs can carry approved + all-deps-done, a state the
        // live system auto-readies immediately. Nothing re-runs that check
        // after import, so normalization must (invariant 4).
        let mut doc = store.export_project(p.id).await.unwrap();
        for task in &mut doc.tasks {
            if task.id == b.id {
                task.status = TaskStatus::Approved;
            }
        }

        let result = store.import_project(&doc).await.unwrap();
        let imported = store
            .list_tasks(result.project_id, None, 100, None, None)
            .await
            .unwrap();
        let status_of = |title: &str| {
            imported
                .items
                .iter()
                .find(|t| t.title == title)
                .unwrap()
                .status
        };
        assert_eq!(status_of("was approved"), TaskStatus::Ready);
    }

    #[tokio::test]
    async fn reject_demotes_when_dep_added_during_review() {
        let store = Store::new_in_memory().await.unwrap();
        // Review gate on: success sends the task to in_review.
        let p = store
            .create_project(&ProjectCreate {
                name: "gated".into(),
                description: None,
                settings: Some(ProjectSettings { review_gate: true }),
            })
            .await
            .unwrap();

        let (t, _claim) = in_progress_task(&store, p.id, "reviewed").await;
        store
            .create_session(
                p.id,
                t.id,
                &session_report(test_identity(), SessionOutcome::Succeeded),
            )
            .await
            .unwrap();
        let t = store.get_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::InReview);

        // A new unfinished dependency lands while the task is in review.
        let dep = ready_task(&store, p.id, "late dep").await;
        store
            .create_relation(
                p.id,
                t.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: dep.id,
                },
            )
            .await
            .unwrap();

        // Rejection must not produce ready-with-undone-deps.
        let t = store.reject_task(p.id, t.id, "needs rework").await.unwrap();
        assert_eq!(t.status, TaskStatus::Approved);
    }

    #[tokio::test]
    async fn unblock_promotes_approved_when_deps_completed_while_blocked() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;

        // A approved with an unfinished dep on B, then blocked.
        let b = ready_task(&store, p.id, "dep").await;
        let a = ready_task(&store, p.id, "blocked while waiting").await;
        store
            .create_relation(
                p.id,
                a.id,
                &RelationCreate {
                    relation_type: RelationType::DependsOn,
                    target_task_id: b.id,
                },
            )
            .await
            .unwrap();
        let a = store.get_task(p.id, a.id).await.unwrap();
        assert_eq!(a.status, TaskStatus::Approved);
        store.block_task(p.id, a.id, "paused").await.unwrap();

        // B completes while A is blocked — the cascade skips blocked tasks.
        let now = Utc::now();
        store
            .claim_task(
                p.id,
                b.id,
                &ClaimRequest {
                    identity: test_identity(),
                    ttl_seconds: 300,
                },
                now,
            )
            .await
            .unwrap();
        store
            .create_session(
                p.id,
                b.id,
                &session_report(test_identity(), SessionOutcome::Succeeded),
            )
            .await
            .unwrap();

        // Unblock must promote A to ready, not strand it approved.
        let a = store.unblock_task(p.id, a.id).await.unwrap();
        assert_eq!(a.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn unblock_demotes_claimless_in_progress() {
        let store = Store::new_in_memory().await.unwrap();
        let p = project_no_gate(&store).await;
        let (t, _claim) = in_progress_task(&store, p.id, "blocked while claimed").await;

        store
            .block_task(p.id, t.id, "waiting on input")
            .await
            .unwrap();
        // The lease expires while blocked; the sweep releases the claim row
        // but leaves the blocked status alone.
        force_expire_claim(&store, t.id).await;
        store.sweep_expired_claims(Utc::now()).await.unwrap();

        // Unblock must not restore a claim-less in_progress.
        let t = store.unblock_task(p.id, t.id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
    }
}
