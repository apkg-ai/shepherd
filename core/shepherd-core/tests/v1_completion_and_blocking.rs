use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::DependencyCreate;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, Epic, EpicCreate, EpicId, Goal, GoalCreate,
    GoalId, Project, ProjectCreate, ProjectId, Task, TaskCreate, TaskId, TestClock,
};
use shepherd_core::queries::{ListParams, WorkFilters, WorkPhase};
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, open, testing};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};
use uuid::Uuid;

async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(testing::store_options(dir.path(), "shepherd.db", clock))
        .await
        .unwrap()
}

async fn independent_connection(path: &Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .unwrap()
}

async fn table_count(conn: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(conn)
        .await
        .unwrap()
}

async fn revision_of(conn: &mut SqliteConnection, table: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT revision FROM {table} WHERE id = ?1"
    )))
    .bind(id.to_string())
    .fetch_one(conn)
    .await
    .unwrap()
}

fn actor(now: DateTime<Utc>, kind: ActorKind, label: &str) -> Actor {
    Actor {
        id: ActorId::generate(now),
        kind,
        label: label.to_string(),
        revoked: false,
        created_at: now,
    }
}

async fn register(store: &Store, actor: &Actor) {
    let inserted = actor.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
        .await
        .unwrap();
}

fn ctx(actor: &Actor, now: DateTime<Utc>, expected_revision: Option<i64>) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        expected_revision,
        now,
    }
}

struct Setup {
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
    owner: Actor,
}

impl Setup {
    fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("shepherd.db")
    }
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = open_store(&dir, clock.clone()).await;
    let owner = actor(clock.now(), ActorKind::Human, "owner");
    register(&store, &owner).await;
    Setup {
        dir,
        clock,
        store,
        owner,
    }
}

async fn create_project(s: &Setup, name: &str) -> Project {
    s.store
        .create_project(
            ctx(&s.owner, s.clock.now(), None),
            ProjectCreate {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

async fn create_goal(s: &Setup, project: ProjectId, title: &str) -> Goal {
    s.store
        .create_goal(
            ctx(&s.owner, s.clock.now(), None),
            project,
            GoalCreate {
                title: title.to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
}

async fn create_epic(s: &Setup, project: ProjectId, goal: GoalId, title: &str) -> Epic {
    s.clock.advance(chrono::TimeDelta::milliseconds(2));
    s.store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project,
            goal,
            EpicCreate {
                title: title.to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
}

async fn create_task(s: &Setup, project: ProjectId, epic: EpicId, title: &str) -> Task {
    s.clock.advance(chrono::TimeDelta::milliseconds(2));
    s.store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project,
            epic,
            TaskCreate {
                title: title.to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

async fn link_epics(s: &Setup, project: ProjectId, dependent: EpicId, prerequisite: EpicId) {
    s.store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project,
            DependencyCreate::Epic {
                dependent_id: dependent,
                prerequisite_id: prerequisite,
            },
        )
        .await
        .unwrap();
}

async fn link_tasks(s: &Setup, project: ProjectId, dependent: TaskId, prerequisite: TaskId) {
    s.store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project,
            DependencyCreate::Task {
                dependent_id: dependent,
                prerequisite_id: prerequisite,
            },
        )
        .await
        .unwrap();
}

async fn complete_task(s: &Setup, project: ProjectId, task: TaskId) {
    let revision = revision_of(
        &mut independent_connection(&s.db_path()).await,
        "tasks",
        task.as_uuid(),
    )
    .await;
    s.store
        .complete_task_for_test(ctx(&s.owner, s.clock.now(), Some(revision)), project, task)
        .await
        .unwrap();
}

async fn epic_state(s: &Setup, epic: EpicId) -> (String, i64) {
    sqlx::query_as("SELECT status, revision FROM epics WHERE id = ?1")
        .bind(epic.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap()
}

/// (type, resource_id, resource_revision, action, reason, command_id).
async fn event_rows(s: &Setup, ids: &[i64]) -> Vec<(String, String, i64, String, String, String)> {
    let mut rows = Vec::new();
    for id in ids {
        rows.push(
            sqlx::query_as(
                "SELECT type, resource_id, resource_revision, action, reason, command_id \
                 FROM events WHERE id = ?1",
            )
            .bind(id)
            .fetch_one(s.store.pool())
            .await
            .unwrap(),
        );
    }
    rows
}

async fn seed_active_claim(s: &Setup, task: TaskId) -> String {
    testing::seed_claim(
        s.store.pool(),
        task,
        s.owner.id,
        "plan",
        "active",
        &format_ts(&s.clock.now()),
        &format_ts(&(s.clock.now() + chrono::Duration::minutes(5))),
    )
    .await
}

async fn seed_expired_claim(s: &Setup, task: TaskId) -> String {
    testing::seed_claim(
        s.store.pool(),
        task,
        s.owner.id,
        "plan",
        "active",
        &format_ts(&s.clock.now()),
        &format_ts(&(s.clock.now() - chrono::Duration::minutes(5))),
    )
    .await
}

async fn seed_pending_submission(s: &Setup, task: TaskId) -> String {
    testing::seed_submission(
        s.store.pool(),
        task,
        s.owner.id,
        "pending",
        &format_ts(&s.clock.now()),
    )
    .await
}

// Acceptance: completing the last required task makes the epic done and
// unlocks its successors in one transaction.
#[tokio::test]
async fn completing_last_required_task_completes_epic_and_unlocks_successors_in_one_transaction() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let first = create_epic(&s, project.id, goal.id, "First").await;
    let first_task = create_task(&s, project.id, first.id, "A").await;
    let second = create_epic(&s, project.id, goal.id, "Second").await;
    let second_task = create_task(&s, project.id, second.id, "B").await;
    link_epics(&s, project.id, second.id, first.id).await;

    // The successor's task waits on the epic prerequisite.
    complete_task(&s, project.id, second_task.id).await;
    assert_eq!(epic_state(&s, second.id).await.0, "open");
    let eligible: Vec<TaskId> = s
        .store
        .list_work(
            &s.owner,
            &project.id,
            WorkPhase::Execute,
            &WorkFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap()
        .items
        .iter()
        .map(|item| item.task.id)
        .collect();
    assert!(eligible.contains(&first_task.id));

    let completed = s
        .store
        .complete_task_for_test(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            first_task.id,
        )
        .await
        .unwrap();

    // One command: the task, its epic and the downstream epic all done, with
    // exactly one coalesced epic.changed per epic and a single command_id.
    assert_eq!(epic_state(&s, first.id).await.0, "done");
    assert_eq!(epic_state(&s, second.id).await.0, "done");
    let rows = event_rows(&s, &completed.events).await;
    assert_eq!(rows.len(), 3);
    let command_ids: std::collections::HashSet<&String> = rows
        .iter()
        .map(|(_, _, _, _, _, command)| command)
        .collect();
    assert_eq!(command_ids.len(), 1);
    for epic_id in [first.id, second.id] {
        let events: Vec<_> = rows
            .iter()
            .filter(|(kind, id, _, _, _, _)| kind == "epic.changed" && *id == epic_id.to_string())
            .collect();
        assert_eq!(events.len(), 1, "one coalesced event per epic");
        let (_, _, event_revision, _, _, _) = events[0];
        assert_eq!(*event_revision, epic_state(&s, epic_id).await.1);
    }
}

// Acceptance: a proposed epic does not auto-complete.
#[tokio::test]
async fn proposed_epic_does_not_auto_complete() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "agent");
    register(&s.store, &agent).await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let proposed = s
        .store
        .create_epic(
            ctx(&agent, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "Proposed".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let task = create_task(&s, project.id, proposed.id, "T").await;

    complete_task(&s, project.id, task.id).await;
    assert_eq!(epic_state(&s, proposed.id).await.0, "proposed");

    // Accepting runs the cascade: the epic completes in the accept command.
    let accepted = s
        .store
        .accept_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            proposed.id,
        )
        .await
        .unwrap();
    assert_eq!(epic_state(&s, proposed.id).await, ("done".into(), 2));
    let rows = event_rows(&s, &accepted.events).await;
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _, _)| kind == "epic.changed")
            .count(),
        1
    );
}

// Acceptance: a blocked epic does not auto-complete, even when children
// finished before the block; unblocking re-evaluates them.
#[tokio::test]
async fn blocked_epic_does_not_auto_complete_until_unblocked() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    s.store
        .block_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            "scope under review".to_string(),
        )
        .await
        .unwrap();

    complete_task(&s, project.id, task.id).await;
    assert_eq!(epic_state(&s, epic.id).await.0, "open");

    let unblocked = s
        .store
        .unblock_epic(ctx(&s.owner, s.clock.now(), Some(2)), project.id, epic.id)
        .await
        .unwrap();
    assert_eq!(epic_state(&s, epic.id).await, ("done".into(), 3));
    let rows = event_rows(&s, &unblocked.events).await;
    // The unblock event preserves the cleared block reason (plan/08).
    assert!(
        rows.iter()
            .all(|(_, _, _, action, reason, _)| action == "unblockEpic"
                && reason == "scope under review")
    );
}

// Acceptance: a cancelled task blocks epic completion until waived.
#[tokio::test]
async fn cancelled_task_blocks_completion_until_waived() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let done = create_task(&s, project.id, epic.id, "Done").await;
    let dropped = create_task(&s, project.id, epic.id, "Dropped").await;

    complete_task(&s, project.id, done.id).await;
    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            dropped.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(epic_state(&s, epic.id).await.0, "open");

    let waived = s
        .store
        .waive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            dropped.id,
            "not needed for the goal".to_string(),
        )
        .await
        .unwrap();
    // Waiving the last cancelled task completes the epic in the same command,
    // reusing the waiver's epic bump: one event, final revision.
    assert_eq!(epic_state(&s, epic.id).await, ("done".into(), 2));
    let rows = event_rows(&s, &waived.events).await;
    let epic_events: Vec<_> = rows
        .iter()
        .filter(|(kind, _, _, _, _, _)| kind == "epic.changed")
        .collect();
    assert_eq!(epic_events.len(), 1);
    assert_eq!(epic_events[0].2, 2);
}

// Acceptance: a waiver does not satisfy a task dependency.
#[tokio::test]
async fn waiver_does_not_satisfy_task_dependency() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let dependent = create_task(&s, project.id, epic.id, "Dependent").await;
    let prerequisite = create_task(&s, project.id, epic.id, "Prerequisite").await;
    link_tasks(&s, project.id, dependent.id, prerequisite.id).await;

    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            prerequisite.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();
    s.store
        .waive_task(
            ctx(&s.owner, s.clock.now(), Some(3)),
            project.id,
            prerequisite.id,
            "excluded from the epic".to_string(),
        )
        .await
        .unwrap();

    // The dependent still waits: cancelled prerequisites stay unmet (plan/04).
    let eligible: Vec<TaskId> = s
        .store
        .list_work(
            &s.owner,
            &project.id,
            WorkPhase::Execute,
            &WorkFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap()
        .items
        .iter()
        .map(|item| item.task.id)
        .collect();
    assert!(!eligible.contains(&dependent.id));
}

// Acceptance: an empty epic requires owner completion.
#[tokio::test]
async fn empty_epic_requires_owner_complete() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "agent");
    register(&s.store, &agent).await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let empty = create_epic(&s, project.id, goal.id, "Empty").await;

    // Never auto-completed on creation, and agents may not complete it.
    assert_eq!(epic_state(&s, empty.id).await.0, "open");
    let err = s
        .store
        .complete_epic(
            ctx(&agent, s.clock.now(), Some(1)),
            project.id,
            empty.id,
            "done".to_string(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    let completed = s
        .store
        .complete_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            empty.id,
            "nothing to deliver".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(epic_state(&s, empty.id).await.0, "done");
    let rows = event_rows(&s, &completed.events).await;
    assert!(
        rows.iter()
            .all(|(_, _, _, action, reason, _)| action == "completeEpic"
                && reason == "nothing to deliver")
    );
}

// Acceptance: an all-waived epic requires owner completion.
#[tokio::test]
async fn all_waived_epic_requires_owner_complete() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let only = create_task(&s, project.id, epic.id, "Only").await;

    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            only.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();
    s.store
        .waive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            only.id,
            "not needed".to_string(),
        )
        .await
        .unwrap();
    // Auto-completion must NOT fire when every task is waived.
    assert_eq!(epic_state(&s, epic.id).await.0, "open");

    s.store
        .complete_epic(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            epic.id,
            "all waived".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(epic_state(&s, epic.id).await.0, "done");
}

// Acceptance: counts still include archived work.
#[tokio::test]
async fn counts_still_include_archived_work() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let other = create_epic(&s, project.id, goal.id, "Other").await;
    link_epics(&s, project.id, other.id, epic.id).await;
    let done = create_task(&s, project.id, epic.id, "Done").await;
    let dropped = create_task(&s, project.id, epic.id, "Dropped").await;

    complete_task(&s, project.id, done.id).await;
    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            dropped.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();
    s.store
        .archive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            done.id,
            "tidy up".to_string(),
        )
        .await
        .unwrap();

    let counted = s.store.get_epic(&project.id, &epic.id).await.unwrap();
    assert_eq!(counted.task_counts.total, 2);
    assert_eq!(counted.task_counts.done, 1);
    assert_eq!(counted.task_counts.cancelled, 1);

    // Archiving the epic keeps counts and dependency edges intact.
    s.store
        .waive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            dropped.id,
            "not needed".to_string(),
        )
        .await
        .unwrap();
    let revision = epic_state(&s, epic.id).await.1;
    s.store
        .archive_epic(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            project.id,
            epic.id,
            "wrapped".to_string(),
        )
        .await
        .unwrap();
    let archived = s.store.get_epic(&project.id, &epic.id).await.unwrap();
    assert!(archived.archived);
    assert_eq!(archived.task_counts.total, 2);
    let mut verify = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut verify, "epic_dependencies").await, 1);
    let goal_counts = s.store.get_goal(&project.id, &goal.id).await.unwrap();
    assert_eq!(goal_counts.epic_counts.total, 2);
}

// Every rejected lifecycle mutation leaves revisions and history untouched.
#[tokio::test]
async fn failed_lifecycle_mutations_leave_revision_and_history_unchanged() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "agent");
    register(&s.store, &agent).await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    let mut verify = independent_connection(&s.db_path()).await;
    let events_before = table_count(&mut verify, "events").await;
    let epic_revision = revision_of(&mut verify, "epics", epic.id.as_uuid()).await;
    let task_revision = revision_of(&mut verify, "tasks", task.id.as_uuid()).await;

    // Stale revision, missing precondition, wrong capability, wrong state and
    // invalid content: each rejection in workflow precedence order.
    let owner = &s.owner;
    let now = s.clock.now();
    let failures: Vec<DomainError> = vec![
        s.store
            .block_task(ctx(owner, now, Some(9)), project.id, task.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .cancel_epic(ctx(owner, now, None), project.id, epic.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .cancel_task(ctx(&agent, now, Some(1)), project.id, task.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .waive_task(ctx(owner, now, Some(1)), project.id, task.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .unblock_epic(ctx(owner, now, Some(1)), project.id, epic.id)
            .await
            .unwrap_err(),
        s.store
            .complete_epic(ctx(owner, now, Some(1)), project.id, epic.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .block_epic(ctx(owner, now, Some(1)), project.id, epic.id, "   ".into())
            .await
            .unwrap_err(),
        s.store
            .archive_task(ctx(owner, now, Some(1)), project.id, task.id, "x".into())
            .await
            .unwrap_err(),
        s.store
            .archive_project(ctx(owner, now, Some(1)), project.id, "x".into())
            .await
            .unwrap_err(),
    ];
    assert!(matches!(failures[0], DomainError::RevisionConflict { .. }));
    assert!(matches!(failures[1], DomainError::PreconditionRequired));
    assert!(matches!(failures[2], DomainError::Forbidden(_)));
    assert!(matches!(failures[3], DomainError::InvalidState(_)));
    assert!(matches!(failures[4], DomainError::InvalidState(_)));
    assert!(matches!(failures[5], DomainError::InvalidState(_)));
    assert!(matches!(
        failures[6],
        DomainError::Validation {
            field: "reason",
            ..
        }
    ));
    assert!(matches!(failures[7], DomainError::InvalidState(_)));
    assert!(matches!(failures[8], DomainError::InvalidState(_)));

    assert_eq!(table_count(&mut verify, "events").await, events_before);
    assert_eq!(
        revision_of(&mut verify, "epics", epic.id.as_uuid()).await,
        epic_revision
    );
    assert_eq!(
        revision_of(&mut verify, "tasks", task.id.as_uuid()).await,
        task_revision
    );
}

// The shared revoke-claims helper works before claim acquisition exists.
#[tokio::test]
async fn block_and_cancel_revoke_seeded_active_claims() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let blocked = create_task(&s, project.id, epic.id, "Blocked").await;
    let cancelled = create_task(&s, project.id, epic.id, "Cancelled").await;
    let blocked_claim = seed_active_claim(&s, blocked.id).await;
    let cancelled_claim = seed_active_claim(&s, cancelled.id).await;
    let blocked_submission = seed_pending_submission(&s, blocked.id).await;
    let cancelled_submission = seed_pending_submission(&s, cancelled.id).await;

    let block = s
        .store
        .block_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            blocked.id,
            "hold".to_string(),
        )
        .await
        .unwrap();
    let cancel = s
        .store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            cancelled.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();

    let mut verify = independent_connection(&s.db_path()).await;
    for (claim, reason) in [(&blocked_claim, "hold"), (&cancelled_claim, "obsolete")] {
        let (status, close_reason, closed_at): (String, String, Option<String>) =
            sqlx::query_as("SELECT status, close_reason, closed_at FROM claims WHERE id = ?1")
                .bind(claim)
                .fetch_one(&mut verify)
                .await
                .unwrap();
        assert_eq!(
            (status.as_str(), close_reason.as_str()),
            ("revoked", reason)
        );
        assert!(closed_at.is_some());
    }
    // Pending submissions survive a block but are withdrawn on cancel.
    let status: String = sqlx::query_scalar("SELECT status FROM submissions WHERE id = ?1")
        .bind(&blocked_submission)
        .fetch_one(&mut verify)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    let (status, withdraw_reason): (String, String) =
        sqlx::query_as("SELECT status, withdraw_reason FROM submissions WHERE id = ?1")
            .bind(&cancelled_submission)
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), withdraw_reason.as_str()),
        ("withdrawn", "obsolete")
    );

    // claim.changed events reference the claim and carry its pinned revision.
    for (result, claim) in [(&block, &blocked_claim), (&cancel, &cancelled_claim)] {
        let rows = event_rows(&s, &result.events).await;
        assert!(rows.iter().any(|(kind, id, revision, _, _, _)| {
            kind == "claim.changed" && id == claim && *revision == 1
        }));
    }
}

// Epic cancellation preserves done descendants and propagates the reason.
#[tokio::test]
async fn epic_cancellation_preserves_done_descendants_and_propagates_reason() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let done = create_task(&s, project.id, epic.id, "Done").await;
    let open = create_task(&s, project.id, epic.id, "Open").await;
    complete_task(&s, project.id, done.id).await;
    let claim = seed_active_claim(&s, open.id).await;

    let cancelled = s
        .store
        .cancel_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            "descoped after review".to_string(),
        )
        .await
        .unwrap();

    let mut verify = independent_connection(&s.db_path()).await;
    let (status, phase, revision): (String, String, i64) =
        sqlx::query_as("SELECT status, phase, revision FROM tasks WHERE id = ?1")
            .bind(done.id.to_string())
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), phase.as_str(), revision),
        ("done", "complete", 2)
    );
    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, cancellation_reason FROM tasks WHERE id = ?1")
            .bind(open.id.to_string())
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    assert_eq!(reason.as_deref(), Some("descoped after review"));
    let claim_status: String = sqlx::query_scalar("SELECT status FROM claims WHERE id = ?1")
        .bind(&claim)
        .fetch_one(&mut verify)
        .await
        .unwrap();
    assert_eq!(claim_status, "revoked");
    let rows = event_rows(&s, &cancelled.events).await;
    assert!(
        rows.iter()
            .all(|(_, _, _, action, reason, _)| action == "cancelEpic"
                && reason == "descoped after review")
    );
    // Done descendants emit nothing; only the cancelled task changes.
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _, _)| kind == "task.changed")
            .count(),
        1
    );
    // Downstream work is still gated: a cancelled epic prerequisite stays unmet.
    let other = create_epic(&s, project.id, goal.id, "Other").await;
    let gated = create_task(&s, project.id, other.id, "Gated").await;
    link_epics(&s, project.id, other.id, epic.id).await;
    complete_task(&s, project.id, gated.id).await;
    assert_eq!(epic_state(&s, other.id).await.0, "open");
}

// Archive requires terminal descendants, freezes the scope, keeps edges.
#[tokio::test]
async fn archive_guards_and_frozen_scope() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    for err in [
        s.store
            .archive_epic(
                ctx(&s.owner, s.clock.now(), Some(1)),
                project.id,
                epic.id,
                "early".to_string(),
            )
            .await
            .unwrap_err(),
        s.store
            .archive_goal(
                ctx(&s.owner, s.clock.now(), Some(1)),
                project.id,
                goal.id,
                "early".to_string(),
            )
            .await
            .unwrap_err(),
        s.store
            .archive_project(
                ctx(&s.owner, s.clock.now(), Some(1)),
                project.id,
                "early".to_string(),
            )
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    complete_task(&s, project.id, task.id).await;
    let submission = seed_pending_submission(&s, task.id).await;
    let err = s
        .store
        .archive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            task.id,
            "tidy".to_string(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));
    sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?1")
        .bind(&submission)
        .execute(s.store.pool())
        .await
        .unwrap();

    s.store
        .archive_epic(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            epic.id,
            "wrapped".to_string(),
        )
        .await
        .unwrap();
    // One-way and frozen: nothing beneath an archived scope mutates.
    let err = s
        .store
        .archive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            task.id,
            "beneath".to_string(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = s
        .store
        .unblock_epic(ctx(&s.owner, s.clock.now(), Some(3)), project.id, epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

// Two independent pools racing cancel against complete: one winner.
#[tokio::test]
async fn cancel_vs_complete_race_yields_single_winner() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    // The second pool shares the database file; the owner already exists.
    let second = open_store(&s.dir, s.clock.clone()).await;

    let cancel = s.store.cancel_task(
        ctx(&s.owner, s.clock.now(), Some(1)),
        project.id,
        task.id,
        "obsolete".to_string(),
    );
    let complete =
        second.complete_task_for_test(ctx(&s.owner, s.clock.now(), Some(1)), project.id, task.id);
    let (cancel_result, complete_result) = tokio::join!(cancel, complete);

    // SQLite write serialization guarantees exactly one winner; the loser is
    // rejected on the terminal re-read inside its own transaction.
    assert!(
        cancel_result.is_ok() != complete_result.is_ok(),
        "exactly one of cancel/complete must win"
    );
    let mut verify = independent_connection(&s.db_path()).await;
    let (status, revision): (String, i64) =
        sqlx::query_as("SELECT status, revision FROM tasks WHERE id = ?1")
            .bind(task.id.to_string())
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert!(status == "cancelled" || status == "done");
    assert_eq!(revision, 2, "the loser wrote nothing");
}

// FLOW-03: creating a task under an epic completed through commands is 409.
#[tokio::test]
async fn creating_task_under_completed_epic_is_terminal() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    complete_task(&s, project.id, task.id).await;
    assert_eq!(epic_state(&s, epic.id).await.0, "done");

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Late".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
}

// Owner-approved extension: deleting the last unmet epic dependency completes
// an otherwise-finished dependent.
#[tokio::test]
async fn deleting_last_unmet_epic_dependency_completes_finished_epic() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let prereq = create_epic(&s, project.id, goal.id, "Prereq").await;
    create_task(&s, project.id, prereq.id, "Keeps it open").await;
    let dependent = create_epic(&s, project.id, goal.id, "Dependent").await;
    let work = create_task(&s, project.id, dependent.id, "Work").await;
    let link = s
        .store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Epic {
                dependent_id: dependent.id,
                prerequisite_id: prereq.id,
            },
        )
        .await
        .unwrap()
        .value;
    complete_task(&s, project.id, work.id).await;
    assert_eq!(epic_state(&s, dependent.id).await.0, "open");

    let deleted = s
        .store
        .delete_dependency(
            ctx(&s.owner, s.clock.now(), Some(link.revision.value())),
            project.id,
            link.id,
        )
        .await
        .unwrap();
    let (status, revision) = epic_state(&s, dependent.id).await;
    assert_eq!(status, "done");
    let rows = event_rows(&s, &deleted.events).await;
    let dependent_events: Vec<_> = rows
        .iter()
        .filter(|(kind, id, _, _, _, _)| kind == "epic.changed" && *id == dependent.id.to_string())
        .collect();
    assert_eq!(dependent_events.len(), 1, "endpoint bump reused, one event");
    assert_eq!(dependent_events[0].2, revision);
}

// Plan/05 step 4: commands reconcile expired claims in scope instead of
// misattributing them to the command reason.
#[tokio::test]
async fn blocking_reconciles_expired_claims_without_misattribution() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let leased = create_task(&s, project.id, epic.id, "Leased").await;
    let lapsed = create_task(&s, project.id, epic.id, "Lapsed").await;
    let live_claim = seed_active_claim(&s, leased.id).await;
    let expired_claim = seed_expired_claim(&s, lapsed.id).await;

    let blocked = s
        .store
        .block_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            "rescoping".to_string(),
        )
        .await
        .unwrap();

    let mut verify = independent_connection(&s.db_path()).await;
    let (status, reason): (String, String) =
        sqlx::query_as("SELECT status, close_reason FROM claims WHERE id = ?1")
            .bind(&live_claim)
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert_eq!((status.as_str(), reason.as_str()), ("revoked", "rescoping"));
    let (status, reason): (String, String) =
        sqlx::query_as("SELECT status, close_reason FROM claims WHERE id = ?1")
            .bind(&expired_claim)
            .fetch_one(&mut verify)
            .await
            .unwrap();
    assert_eq!((status.as_str(), reason.as_str()), ("expired", ""));
    // Only the truly revoked lease changed its task's representation.
    assert_eq!(
        revision_of(&mut verify, "tasks", leased.id.as_uuid()).await,
        2
    );
    assert_eq!(
        revision_of(&mut verify, "tasks", lapsed.id.as_uuid()).await,
        1
    );
    let rows = event_rows(&s, &blocked.events).await;
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _, _)| kind == "claim.changed")
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _, _)| kind == "task.changed")
            .count(),
        1
    );
}

// Owner-confirmed spec-literal behavior (plan/04 unblock row: "owner; block
// exists"): cancel retains the block record and unblock may clear it even on
// terminal work.
#[tokio::test]
async fn unblock_clears_stale_block_on_cancelled_task() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    s.store
        .block_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            "waiting on design".to_string(),
        )
        .await
        .unwrap();
    let cancelled = s
        .store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project.id,
            task.id,
            "obsolete".to_string(),
        )
        .await
        .unwrap();
    // Cancel keeps the durable block record on the terminal task.
    assert!(cancelled.value.block.is_some());

    let unblocked = s
        .store
        .unblock_task(ctx(&s.owner, s.clock.now(), Some(3)), project.id, task.id)
        .await
        .unwrap();
    assert!(unblocked.value.block.is_none());
    assert_eq!(unblocked.value.status.as_str(), "cancelled");
    assert_eq!(unblocked.value.revision.value(), 4);
    let rows = event_rows(&s, &unblocked.events).await;
    assert!(rows.iter().all(
        |(_, _, _, action, reason, _)| action == "unblockTask" && reason == "waiting on design"
    ));
}

// A linear epic chain cascades one wave per depth level; the completion of
// the chain's head task must finish every epic in the single command, with
// each epic bumped exactly once (revision 2) — the set-based pending scan.
#[tokio::test]
async fn deep_dependency_chain_cascades_with_one_bump_per_epic() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let depth = 100;

    let mut epics = Vec::new();
    for index in 0..depth {
        epics.push(create_epic(&s, project.id, goal.id, &format!("E{index}")).await);
    }
    let mut tasks = Vec::new();
    for epic in &epics {
        tasks.push(create_task(&s, project.id, epic.id, "T").await);
    }
    // Epic i depends on epic i+1: only the tail has no live prerequisite.
    for pair in epics.windows(2) {
        link_epics(&s, project.id, pair[0].id, pair[1].id).await;
    }

    // Completing every task but the tail's leaves all epics blocked.
    for task in &tasks[..depth - 1] {
        complete_task(&s, project.id, task.id).await;
    }
    for epic in &epics {
        assert_eq!(epic_state(&s, epic.id).await.0, "open");
    }

    // The tail's task completes the tail epic and the cascade walks the
    // whole chain in this one command, bumping each epic exactly once.
    let mut before = Vec::new();
    for epic in &epics {
        before.push(epic_state(&s, epic.id).await.1);
    }
    complete_task(&s, project.id, tasks[depth - 1].id).await;
    for (epic, before) in epics.iter().zip(before) {
        assert_eq!(
            epic_state(&s, epic.id).await,
            ("done".to_string(), before + 1),
            "epic {} must complete with exactly one bump",
            epic.id
        );
    }
}
