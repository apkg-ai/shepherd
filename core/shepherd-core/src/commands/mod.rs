mod dependencies;
mod hierarchy;

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqliteConnection, Transaction};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Actor, ActorId, ActorKind, CommandId, DependencyId, EpicId, EventId, GoalId, ProjectId,
    Revision, TaskId, TaskTypeId,
};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store, rows};

pub struct CommandContext {
    pub actor: Actor,
    pub command_id: CommandId,
    /// Carried per plan/05; replay lookup lands with the real codec in step 007.
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

pub(crate) fn require_owner(actor: &Actor) -> Result<(), DomainError> {
    match actor.kind {
        ActorKind::Human => Ok(()),
        ActorKind::Agent => Err(DomainError::Forbidden("owner capability required".into())),
    }
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

#[cfg_attr(not(test), expect(dead_code))] // used from workflow transitions later in step 006
pub(crate) enum ClaimScope {
    Task(TaskId),
    Epic(EpicId),
}

#[cfg_attr(not(test), expect(dead_code))]
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

#[cfg_attr(not(test), expect(dead_code))]
pub(crate) struct RevokedClaim {
    pub(crate) id: Uuid,
    pub(crate) task_id: TaskId,
    pub(crate) task_revision: i64,
}

// Closes every active claim in scope regardless of expiry; expired-claim
// reconciliation semantics land with the claim model in step 009.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) async fn revoke_active_claims(
    conn: &mut SqliteConnection,
    scope: ClaimScope,
    now: &str,
    close_reason: &str,
) -> Result<Vec<RevokedClaim>, DomainError> {
    let query = sqlx::AssertSqlSafe(format!(
        "UPDATE claims SET status = 'revoked', closed_at = ?2, close_reason = ?3 \
         WHERE status = 'active' AND {} RETURNING id, task_id, task_revision",
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
            Ok(RevokedClaim {
                id: rows::parse_uuid("claims.id", &id)?,
                task_id: TaskId::from_uuid(rows::parse_uuid("claims.task_id", &task_id)?),
                task_revision,
            })
        })
        .collect()
}

#[cfg_attr(not(test), expect(dead_code))]
pub(crate) struct WithdrawnSubmission {
    pub(crate) id: Uuid,
    pub(crate) revision: i64,
    pub(crate) task_id: TaskId,
}

#[cfg_attr(not(test), expect(dead_code))]
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
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn claim(id: Uuid, task_revision: i64, task: TaskId) -> Self {
        Self {
            event_type: "claim.changed",
            kind_rank: 6,
            resource_id: id,
            resource_revision: task_revision,
            affected_ids: vec![task.as_uuid()],
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
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

    fn actor(kind: ActorKind, revoked: bool) -> Actor {
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        Actor {
            id: ActorId::generate(now),
            kind,
            label: "someone".to_string(),
            revoked,
            created_at: now,
        }
    }

    #[test]
    fn only_humans_hold_the_owner_capability() {
        assert!(require_owner(&actor(ActorKind::Human, false)).is_ok());
        assert!(matches!(
            require_owner(&actor(ActorKind::Agent, false)),
            Err(DomainError::Forbidden(_))
        ));
    }

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
        ActorId, Clock, EpicCreate, EpicId, GoalCreate, ProjectCreate, TaskCreate, TaskId,
        TestClock,
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

    // Direct-SQL fixture: no public command can claim until step 009.
    async fn seed_claim(f: &Fixture, task: TaskId, status: &str, expires_at: &str) -> String {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
             status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'execute', ?4, ?5, ?6, \
             1, 'hash')",
        )
        .bind(&id)
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(expires_at)
        .bind(status)
        .execute(f.store.pool())
        .await
        .unwrap();
        id
    }

    // Direct-SQL fixture: sessions/submissions get commands in steps 010/011.
    async fn seed_submission(f: &Fixture, task: TaskId, status: &str) -> String {
        let now = format_ts(&f.clock.now());
        let session_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let mut tx = f.store.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, task_id, claim_id, actor_id, phase, started_at, \
             ended_at, outcome, summary, failure_reason, document_revision_ids, links) \
             VALUES (?1, ?2, 'claim', ?3, 'execute', ?4, ?4, 'succeeded', '', '', '[]', '[]')",
        )
        .bind(&session_id)
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO submissions (id, revision, created_at, updated_at, task_id, kind, \
             producer_id, document_revision_ids, session_id, policy, status, \
             created_context_revision) VALUES (?1, 1, ?2, ?2, ?3, 'work', ?4, '[]', ?5, \
             'human', ?6, 1)",
        )
        .bind(&id)
        .bind(&now)
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(&session_id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn claim_row(f: &Fixture, id: &str) -> (String, Option<String>, String) {
        sqlx::query_as("SELECT status, closed_at, close_reason FROM claims WHERE id = ?1")
            .bind(id)
            .fetch_one(f.store.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn revoke_active_claims_scopes_to_task_or_epic_and_skips_closed_rows() {
        let f = fixture().await;
        let now = format_ts(&f.clock.now());
        let future = "2027-01-01T00:00:00.000Z";
        let active_a = seed_claim(&f, f.task_a, "active", future).await;
        let released_a = seed_claim(&f, f.task_a, "released", future).await;
        // Active-but-expired rows are still revoked; reconciliation is step 009.
        let expired_b = seed_claim(&f, f.task_b, "active", "2020-01-01T00:00:00.000Z").await;

        let task_a = f.task_a;
        let stamp = now.clone();
        let revoked = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    revoke_active_claims(tx, ClaimScope::Task(task_a), &stamp, "blocked").await
                })
            })
            .await
            .unwrap();
        assert_eq!(revoked.len(), 1);
        assert_eq!(revoked[0].id.to_string(), active_a);
        assert_eq!(revoked[0].task_id, f.task_a);
        assert_eq!(revoked[0].task_revision, 1);
        let (status, closed_at, close_reason) = claim_row(&f, &active_a).await;
        assert_eq!(
            (status.as_str(), close_reason.as_str()),
            ("revoked", "blocked")
        );
        assert_eq!(closed_at.as_deref(), Some(now.as_str()));
        assert_eq!(claim_row(&f, &released_a).await.0, "released");
        assert_eq!(claim_row(&f, &expired_b).await.0, "active");

        let epic = f.epic;
        let stamp = now.clone();
        let revoked = f
            .store
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    revoke_active_claims(tx, ClaimScope::Epic(epic), &stamp, "cancelled").await
                })
            })
            .await
            .unwrap();
        assert_eq!(revoked.len(), 1);
        assert_eq!(revoked[0].id.to_string(), expired_b);
        assert_eq!(revoked[0].task_id, f.task_b);
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
