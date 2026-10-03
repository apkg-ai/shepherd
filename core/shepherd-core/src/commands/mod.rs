mod archive;
mod claims;
mod dependencies;
mod hierarchy;
mod identity;

pub use identity::IdentityPaths;

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqliteConnection, Transaction};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Actor, ActorId, CommandId, DependencyId, Epic, EpicId, EventId, GoalId, IDEMPOTENCY_TTL_DAYS,
    ProjectId, Revision, Task, TaskId, TaskTypeId,
};
use crate::queries::hierarchy::{epic_row, goal_row, project_archived, task_row};
use crate::storage::rows::format_ts;
use crate::storage::{IdempotencyAad, StorageError, Store, rows};

pub struct CommandContext {
    pub actor: Actor,
    pub command_id: CommandId,
    /// Honored by idempotent_transaction; plain domain_transaction commands are not replayable.
    pub idempotency_key: Uuid,
    pub expected_revision: Option<i64>,
    pub now: DateTime<Utc>,
}

#[derive(Debug)]
pub struct CommandResult<T> {
    pub value: T,
    pub events: Vec<EventId>,
}

pub(crate) type DomainTxFuture<'t, T> =
    Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 't>>;

/// Whether the value was computed by this call or decrypted from the stored
/// idempotency response (plan/05 step 2).
#[derive(Debug)]
pub enum Replay<T> {
    Fresh(T),
    Replayed(T),
}

impl<T> Replay<T> {
    pub fn into_inner(self) -> T {
        match self {
            Replay::Fresh(value) | Replay::Replayed(value) => value,
        }
    }

    pub fn is_replay(&self) -> bool {
        matches!(self, Replay::Replayed(_))
    }
}

impl Store {
    pub(crate) async fn domain_transaction<T, F>(&self, command: F) -> Result<T, DomainError>
    where
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> DomainTxFuture<'t, T>,
    {
        let mut tx = self.begin_command().await?;
        match command(&mut tx).await {
            Ok(value) => {
                tx.commit().await.map_err(StorageError::from)?;
                Ok(value)
            }
            Err(err) => {
                tx.rollback().await.ok();
                Err(err)
            }
        }
    }

    // Plan/05 command algorithm step 2: revocation check strictly before the
    // replay lookup, encrypted response stored in the same transaction.
    pub(crate) async fn idempotent_transaction<T, F>(
        &self,
        ctx: &CommandContext,
        request_hash: &str,
        response_status: i64,
        command: F,
    ) -> Result<Replay<T>, DomainError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> DomainTxFuture<'t, T>,
    {
        let mut tx = self.begin_command().await?;
        match self
            .idempotent_body(&mut tx, ctx, request_hash, response_status, command)
            .await
        {
            Ok(value) => {
                tx.commit().await.map_err(StorageError::from)?;
                Ok(value)
            }
            Err(err) => {
                tx.rollback().await.ok();
                Err(err)
            }
        }
    }

    async fn idempotent_body<T, F>(
        &self,
        tx: &mut Transaction<'static, Sqlite>,
        ctx: &CommandContext,
        request_hash: &str,
        response_status: i64,
        command: F,
    ) -> Result<Replay<T>, DomainError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> DomainTxFuture<'t, T>,
    {
        live_actor(tx, &ctx.actor.id).await?;
        let aad = IdempotencyAad {
            actor_id: &ctx.actor.id,
            key: &ctx.idempotency_key,
            request_hash,
        };
        if let Some(record) =
            rows::get_idempotency(&mut **tx, &ctx.actor.id, &ctx.idempotency_key, ctx.now).await?
        {
            if record.request_hash != request_hash {
                return Err(DomainError::IdempotencyConflict);
            }
            let plaintext = self.codec().open(&aad, &record.sealed)?;
            let value: T = serde_json::from_slice(&plaintext)
                .map_err(|err| StorageError::Corrupt(format!("idempotency response: {err}")))?;
            return Ok(Replay::Replayed(value));
        }
        let value = command(tx).await?;
        let plaintext = serde_json::to_vec(&value)
            .map_err(|err| StorageError::Corrupt(format!("idempotency response: {err}")))?;
        let sealed = self.codec().seal(&aad, &plaintext)?;
        rows::put_idempotency(
            tx,
            &rows::IdempotencyRecord {
                actor_id: ctx.actor.id,
                key: ctx.idempotency_key,
                command_id: ctx.command_id,
                request_hash: request_hash.to_string(),
                response_status,
                sealed,
                created_at: ctx.now,
                expires_at: ctx.now + chrono::TimeDelta::days(IDEMPOTENCY_TTL_DAYS),
            },
        )
        .await?;
        Ok(Replay::Fresh(value))
    }
}

// Authenticated callers are rechecked inside the transaction (plan/05).
pub(crate) async fn live_actor(
    conn: &mut SqliteConnection,
    id: &ActorId,
) -> Result<Actor, DomainError> {
    match rows::get_actor(&mut *conn, id).await? {
        Some(actor) if !actor.revoked => Ok(actor),
        _ => Err(DomainError::Forbidden(
            "actor is not registered or is revoked".into(),
        )),
    }
}

// Shared scope loaders: membership first (404), archived-chain walk deferred
// so commands keep the plan/04 precedence (capability and revision before
// archived scope).
pub(crate) struct TaskScope {
    pub(crate) task: Task,
    pub(crate) epic: Epic,
    goal_archived: bool,
    project_archived: bool,
}

pub(crate) struct EpicScope {
    pub(crate) epic: Epic,
    goal_archived: bool,
    project_archived: bool,
}

impl TaskScope {
    pub(crate) fn ensure_unarchived(&self) -> Result<(), DomainError> {
        if self.task.archived || self.epic.archived || self.goal_archived || self.project_archived {
            return Err(DomainError::ArchivedScope);
        }
        Ok(())
    }
}

impl EpicScope {
    pub(crate) fn ensure_unarchived(&self) -> Result<(), DomainError> {
        if self.epic.archived || self.goal_archived || self.project_archived {
            return Err(DomainError::ArchivedScope);
        }
        Ok(())
    }
}

pub(crate) async fn task_scope(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
) -> Result<TaskScope, DomainError> {
    let task = task_row(conn, project, task)
        .await?
        .ok_or(DomainError::NotFound)?;
    let epic = epic_row(conn, project, &task.epic_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    let goal = goal_row(conn, project, &epic.goal_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    Ok(TaskScope {
        task,
        epic,
        goal_archived: goal.archived,
        project_archived: project_archived(conn, project).await?,
    })
}

pub(crate) async fn epic_scope(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    epic: &EpicId,
) -> Result<EpicScope, DomainError> {
    let epic = epic_row(conn, project, epic)
        .await?
        .ok_or(DomainError::NotFound)?;
    let goal = goal_row(conn, project, &epic.goal_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    Ok(EpicScope {
        epic,
        goal_archived: goal.archived,
        project_archived: project_archived(conn, project).await?,
    })
}

pub(crate) fn missing_after_write(what: &'static str) -> DomainError {
    StorageError::Corrupt(format!("{what} missing after write")).into()
}

pub(crate) fn require_revision(expected: Option<i64>, actual: Revision) -> Result<(), DomainError> {
    match expected {
        None => Err(DomainError::PreconditionRequired),
        Some(value) if value == actual.value() => Ok(()),
        Some(value) => Err(DomainError::RevisionConflict {
            expected: value,
            actual: actual.value(),
        }),
    }
}

// Unexpired active claim or pending submission blocks mutations on the
// resource; the epic level checks every descendant task (plan/03, plan/04
// FLOW-03).
// Single source for "active work on task alias `t`": an unexpired active claim
// or a pending submission (plan/03, plan/04 FLOW-03).
pub(crate) fn active_work_predicate(now_placeholder: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM claims c WHERE c.task_id = t.id \
         AND c.status = 'active' AND c.expires_at > {now_placeholder}) \
         OR EXISTS (SELECT 1 FROM submissions s WHERE s.task_id = t.id \
         AND s.status = 'pending'))"
    )
}

async fn has_active_work_by(
    conn: &mut SqliteConnection,
    column: &'static str,
    resource: Uuid,
    now: &str,
) -> Result<bool, DomainError> {
    let query = sqlx::AssertSqlSafe(format!(
        "SELECT 1 FROM tasks t WHERE t.{column} = ?1 AND {} LIMIT 1",
        active_work_predicate("?2")
    ));
    Ok(sqlx::query(query)
        .bind(resource.to_string())
        .bind(now)
        .fetch_optional(&mut *conn)
        .await?
        .is_some())
}

pub(crate) async fn task_has_active_work(
    conn: &mut SqliteConnection,
    task: TaskId,
    now: &str,
) -> Result<bool, DomainError> {
    has_active_work_by(conn, "id", task.as_uuid(), now).await
}

pub(crate) async fn epic_has_active_work(
    conn: &mut SqliteConnection,
    epic: EpicId,
    now: &str,
) -> Result<bool, DomainError> {
    has_active_work_by(conn, "epic_id", epic.as_uuid(), now).await
}

pub(crate) enum ClaimScope {
    Task(TaskId),
    Epic(EpicId),
}

impl ClaimScope {
    fn task_filter(&self) -> &'static str {
        match self {
            ClaimScope::Task(_) => "task_id = ?1",
            ClaimScope::Epic(_) => "task_id IN (SELECT id FROM tasks WHERE epic_id = ?1)",
        }
    }

    fn id(&self) -> Uuid {
        match self {
            ClaimScope::Task(task) => task.as_uuid(),
            ClaimScope::Epic(epic) => epic.as_uuid(),
        }
    }
}

pub(crate) struct ClosedClaim {
    pub(crate) id: Uuid,
    pub(crate) task_id: TaskId,
    pub(crate) task_revision: i64,
}

pub(crate) struct ClosedClaims {
    // Reconciled leases: they were already inactive for reads, so their tasks
    // did not change representation.
    pub(crate) expired: Vec<ClosedClaim>,
    pub(crate) revoked: Vec<ClosedClaim>,
}

impl ClosedClaims {
    pub(crate) fn all(&self) -> impl Iterator<Item = &ClosedClaim> {
        self.expired.iter().chain(self.revoked.iter())
    }
}

// Plan/05 step 4: reconcile expired claims in the affected scope first, then
// revoke the still-live ones with the command reason.
pub(crate) async fn revoke_active_claims(
    conn: &mut SqliteConnection,
    scope: ClaimScope,
    now: &str,
    close_reason: &str,
) -> Result<ClosedClaims, DomainError> {
    let expired = close_claims(conn, &scope, now, "expired", "expires_at <= ?2", "").await?;
    let revoked = close_claims(
        conn,
        &scope,
        now,
        "revoked",
        "expires_at > ?2",
        close_reason,
    )
    .await?;
    Ok(ClosedClaims { expired, revoked })
}

async fn close_claims(
    conn: &mut SqliteConnection,
    scope: &ClaimScope,
    now: &str,
    status: &'static str,
    expiry_filter: &'static str,
    close_reason: &str,
) -> Result<Vec<ClosedClaim>, DomainError> {
    let query = sqlx::AssertSqlSafe(format!(
        "UPDATE claims SET status = '{status}', closed_at = ?2, close_reason = ?3 \
         WHERE status = 'active' AND {expiry_filter} AND {} \
         RETURNING id, task_id, task_revision",
        scope.task_filter()
    ));
    let rows: Vec<(String, String, i64)> = sqlx::query_as(query)
        .bind(scope.id().to_string())
        .bind(now)
        .bind(close_reason)
        .fetch_all(&mut *conn)
        .await?;
    rows.into_iter()
        .map(|(id, task_id, task_revision)| {
            Ok(ClosedClaim {
                id: rows::parse_uuid("claims.id", &id)?,
                task_id: TaskId::from_uuid(rows::parse_uuid("claims.task_id", &task_id)?),
                task_revision,
            })
        })
        .collect()
}

pub(crate) struct WithdrawnSubmission {
    pub(crate) id: Uuid,
    pub(crate) revision: i64,
    pub(crate) task_id: TaskId,
}

pub(crate) async fn withdraw_pending_submissions(
    conn: &mut SqliteConnection,
    scope: ClaimScope,
    now: &str,
    reason: &str,
) -> Result<Vec<WithdrawnSubmission>, DomainError> {
    let query = sqlx::AssertSqlSafe(format!(
        "UPDATE submissions SET status = 'withdrawn', withdraw_reason = ?3, \
         revision = revision + 1, updated_at = ?2 \
         WHERE status = 'pending' AND {} RETURNING id, revision, task_id",
        scope.task_filter()
    ));
    let rows: Vec<(String, i64, String)> = sqlx::query_as(query)
        .bind(scope.id().to_string())
        .bind(now)
        .bind(reason)
        .fetch_all(&mut *conn)
        .await?;
    rows.into_iter()
        .map(|(id, revision, task_id)| {
            Ok(WithdrawnSubmission {
                id: rows::parse_uuid("submissions.id", &id)?,
                revision,
                task_id: TaskId::from_uuid(rows::parse_uuid("submissions.task_id", &task_id)?),
            })
        })
        .collect()
}

pub(crate) struct PendingEvent {
    pub(crate) event_type: &'static str,
    // Ownership order (project, goal, epic, task, registry) for deterministic event rows.
    kind_rank: u8,
    pub(crate) resource_id: Uuid,
    pub(crate) resource_revision: i64,
    affected_ids: Vec<Uuid>,
}

impl PendingEvent {
    pub(crate) fn project(id: ProjectId, revision: Revision) -> Self {
        Self {
            event_type: "project.changed",
            kind_rank: 0,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    pub(crate) fn goal(id: GoalId, revision: Revision) -> Self {
        Self {
            event_type: "goal.changed",
            kind_rank: 1,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    // epic/task changes carry the owning goal/epic id in affected_ids (plan/08).
    pub(crate) fn epic(id: EpicId, revision: Revision, goal: GoalId) -> Self {
        Self {
            event_type: "epic.changed",
            kind_rank: 2,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: vec![goal.as_uuid()],
        }
    }

    pub(crate) fn task(id: TaskId, revision: Revision, epic: EpicId) -> Self {
        Self {
            event_type: "task.changed",
            kind_rank: 3,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: vec![epic.as_uuid()],
        }
    }

    pub(crate) fn task_type(id: TaskTypeId, revision: Revision) -> Self {
        Self {
            event_type: "task_type.changed",
            kind_rank: 4,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    // affected carries [dependent, prerequisite, scope] (plan/08: both endpoints
    // and affected scope).
    pub(crate) fn dependency(id: DependencyId, revision: Revision, affected: Vec<Uuid>) -> Self {
        Self {
            event_type: "dependency.changed",
            kind_rank: 5,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: affected,
        }
    }

    // Claims have no revision column; the event carries the task revision
    // pinned at acquisition (revisit when the claim model lands in step 009).
    pub(crate) fn claim(id: Uuid, task_revision: i64, task: TaskId) -> Self {
        Self {
            event_type: "claim.changed",
            kind_rank: 6,
            resource_id: id,
            resource_revision: task_revision,
            affected_ids: vec![task.as_uuid()],
        }
    }

    pub(crate) fn submission(id: Uuid, revision: i64, task: TaskId) -> Self {
        Self {
            event_type: "submission.changed",
            kind_rank: 7,
            resource_id: id,
            resource_revision: revision,
            affected_ids: vec![task.as_uuid()],
        }
    }
}

// `reason` is the command reason or empty string; durable explanations for
// block/cancel/waive/archive and cleared-block reasons live here (plan/08).
pub(crate) async fn append_events(
    conn: &mut SqliteConnection,
    project_id: &ProjectId,
    ctx: &CommandContext,
    action: &'static str,
    reason: &str,
    mut events: Vec<PendingEvent>,
) -> Result<Vec<EventId>, DomainError> {
    // Deterministic order within a command: resource kind, then UUID (plan/08).
    events.sort_by_key(|event| (event.kind_rank, event.resource_id));
    let occurred_at = format_ts(&ctx.now);
    let mut ids = Vec::with_capacity(events.len());
    for event in &events {
        let affected: Vec<String> = event.affected_ids.iter().map(Uuid::to_string).collect();
        let affected_json = serde_json::to_string(&affected)
            .map_err(|err| StorageError::Corrupt(err.to_string()))?;
        let inserted = sqlx::query(
            "INSERT INTO events (action, reason, project_id, actor_id, command_id, type, \
             resource_id, resource_revision, occurred_at, affected_ids) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(action)
        .bind(reason)
        .bind(project_id.to_string())
        .bind(ctx.actor.id.to_string())
        .bind(ctx.command_id.to_string())
        .bind(event.event_type)
        .bind(event.resource_id.to_string())
        .bind(event.resource_revision)
        .bind(&occurred_at)
        .bind(&affected_json)
        .execute(&mut *conn)
        .await?;
        ids.push(inserted.last_insert_rowid());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_checks_follow_the_precondition_contract() {
        assert!(require_revision(Some(1), Revision::INITIAL).is_ok());
        assert!(matches!(
            require_revision(None, Revision::INITIAL),
            Err(DomainError::PreconditionRequired)
        ));
        assert!(matches!(
            require_revision(Some(2), Revision::INITIAL),
            Err(DomainError::RevisionConflict {
                expected: 2,
                actual: 1
            })
        ));
    }

    #[test]
    fn pending_events_sort_by_kind_then_uuid() {
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        let project = ProjectId::generate(now);
        let first_type = TaskTypeId::generate(now);
        let second_type = TaskTypeId::generate(now);
        let task = TaskId::generate(now);
        let mut events = [
            PendingEvent::submission(Uuid::nil(), 1, task),
            PendingEvent::claim(Uuid::nil(), 1, task),
            PendingEvent::task_type(second_type, Revision::INITIAL),
            PendingEvent::task_type(first_type, Revision::INITIAL),
            PendingEvent::project(project, Revision::INITIAL),
        ];
        events.sort_by_key(|event| (event.kind_rank, event.resource_id));
        assert_eq!(events[0].event_type, "project.changed");
        assert_eq!(events[1].resource_id, first_type.as_uuid());
        assert_eq!(events[2].resource_id, second_type.as_uuid());
        assert_eq!(events[3].event_type, "claim.changed");
        assert_eq!(events[4].event_type, "submission.changed");
    }

    #[test]
    fn claim_and_submission_events_reference_the_owning_task() {
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        let task = TaskId::generate(now);
        let claim = PendingEvent::claim(Uuid::nil(), 7, task);
        assert_eq!(claim.resource_revision, 7);
        assert_eq!(claim.affected_ids, vec![task.as_uuid()]);
        let submission = PendingEvent::submission(Uuid::nil(), 2, task);
        assert_eq!(submission.resource_revision, 2);
        assert_eq!(submission.affected_ids, vec![task.as_uuid()]);
    }
}

#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::model::{
        ActorId, ActorKind, Clock, EpicCreate, EpicId, GoalCreate, ProjectCreate, TaskCreate,
        TaskId, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        epic: EpicId,
        task_a: TaskId,
        task_b: TaskId,
    }

    fn ctx(actor: &Actor, clock: &TestClock) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision: None,
            now: clock.now(),
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = Actor {
            id: ActorId::generate(clock.now()),
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
                ctx(&owner, &clock),
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
                ctx(&owner, &clock),
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
                ctx(&owner, &clock),
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
        let mut tasks = Vec::new();
        for title in ["A", "B"] {
            tasks.push(
                store
                    .create_task(
                        ctx(&owner, &clock),
                        project,
                        epic,
                        TaskCreate {
                            title: title.to_string(),
                            description: None,
                            type_key: "code".to_string(),
                            planning_required: None,
                            plan_review: None,
                            work_review: None,
                        },
                    )
                    .await
                    .unwrap()
                    .value
                    .id,
            );
        }
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            epic,
            task_a: tasks[0],
            task_b: tasks[1],
        }
    }

    async fn seed_claim(f: &Fixture, task: TaskId, status: &str, expires_at: &str) -> String {
        crate::storage::testing::seed_claim(
            f.store.pool(),
            task,
            f.owner.id,
            "execute",
            status,
            &format_ts(&f.clock.now()),
            expires_at,
        )
        .await
    }

    async fn seed_submission(f: &Fixture, task: TaskId, status: &str) -> String {
        crate::storage::testing::seed_submission(
            f.store.pool(),
            task,
            f.owner.id,
            status,
            &format_ts(&f.clock.now()),
        )
        .await
    }

    async fn claim_row(f: &Fixture, id: &str) -> (String, Option<String>, String) {
        sqlx::query_as("SELECT status, closed_at, close_reason FROM claims WHERE id = ?1")
            .bind(id)
            .fetch_one(f.store.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn revoke_active_claims_scopes_reconciles_expiry_and_skips_closed_rows() {
        let f = fixture().await;
        let now = format_ts(&f.clock.now());
        let future = "2027-01-01T00:00:00.000Z";
        let active_a = seed_claim(&f, f.task_a, "active", future).await;
        let released_a = seed_claim(&f, f.task_a, "released", future).await;
        let expired_b = seed_claim(&f, f.task_b, "active", "2020-01-01T00:00:00.000Z").await;

        let task_a = f.task_a;
        let stamp = now.clone();
        let closed = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    revoke_active_claims(tx, ClaimScope::Task(task_a), &stamp, "blocked").await
                })
            })
            .await
            .unwrap();
        assert!(closed.expired.is_empty());
        assert_eq!(closed.revoked.len(), 1);
        assert_eq!(closed.revoked[0].id.to_string(), active_a);
        assert_eq!(closed.revoked[0].task_id, f.task_a);
        assert_eq!(closed.revoked[0].task_revision, 1);
        let (status, closed_at, close_reason) = claim_row(&f, &active_a).await;
        assert_eq!(
            (status.as_str(), close_reason.as_str()),
            ("revoked", "blocked")
        );
        assert_eq!(closed_at.as_deref(), Some(now.as_str()));
        assert_eq!(claim_row(&f, &released_a).await.0, "released");
        assert_eq!(claim_row(&f, &expired_b).await.0, "active");

        // Plan/05 step 4: an already-expired lease is reconciled, never
        // misattributed to the command reason.
        let epic = f.epic;
        let stamp = now.clone();
        let closed = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    revoke_active_claims(tx, ClaimScope::Epic(epic), &stamp, "cancelled").await
                })
            })
            .await
            .unwrap();
        assert!(closed.revoked.is_empty());
        assert_eq!(closed.expired.len(), 1);
        assert_eq!(closed.expired[0].id.to_string(), expired_b);
        assert_eq!(closed.expired[0].task_id, f.task_b);
        let (status, closed_at, close_reason) = claim_row(&f, &expired_b).await;
        assert_eq!((status.as_str(), close_reason.as_str()), ("expired", ""));
        assert_eq!(closed_at.as_deref(), Some(now.as_str()));
    }

    #[tokio::test]
    async fn withdraw_pending_submissions_bumps_and_skips_settled_rows() {
        let f = fixture().await;
        let now = format_ts(&f.clock.now());
        let pending_a = seed_submission(&f, f.task_a, "pending").await;
        let accepted_b = seed_submission(&f, f.task_b, "accepted").await;

        let epic = f.epic;
        let stamp = now.clone();
        let withdrawn = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    withdraw_pending_submissions(tx, ClaimScope::Epic(epic), &stamp, "cancelled")
                        .await
                })
            })
            .await
            .unwrap();
        assert_eq!(withdrawn.len(), 1);
        assert_eq!(withdrawn[0].id.to_string(), pending_a);
        assert_eq!(withdrawn[0].revision, 2);
        assert_eq!(withdrawn[0].task_id, f.task_a);
        let (status, revision, reason): (String, i64, String) = sqlx::query_as(
            "SELECT status, revision, withdraw_reason FROM submissions WHERE id = ?1",
        )
        .bind(&pending_a)
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(
            (status.as_str(), revision, reason.as_str()),
            ("withdrawn", 2, "cancelled")
        );
        let untouched: String = sqlx::query_scalar("SELECT status FROM submissions WHERE id = ?1")
            .bind(&accepted_b)
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(untouched, "accepted");
    }

    #[tokio::test]
    async fn append_events_persists_the_command_reason() {
        let f = fixture().await;
        let command = ctx(&f.owner, &f.clock);
        let project = f.project;
        let epic_event = PendingEvent::claim(Uuid::nil(), 1, f.task_a);
        f.store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    append_events(
                        tx,
                        &project,
                        &command,
                        "blockTask",
                        "why not",
                        vec![epic_event],
                    )
                    .await
                })
            })
            .await
            .unwrap();
        let (action, reason, event_type): (String, String, String) =
            sqlx::query_as("SELECT action, reason, type FROM events ORDER BY id DESC LIMIT 1")
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(
            (action.as_str(), reason.as_str(), event_type.as_str()),
            ("blockTask", "why not", "claim.changed")
        );
    }
}
