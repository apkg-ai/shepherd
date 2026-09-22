use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, Counts, Epic, EpicCreate, EpicId, EpicStatus,
    Goal, GoalCreate, GoalId, Project, ProjectCreate, ProjectId, ProjectPatch, ProjectSettings,
    ReviewPolicy, Task, TaskCreate, TaskId, TaskPatch, TaskPhase, TaskStatus, TestClock, TextPatch,
};
use shepherd_core::queries::ListParams;
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

async fn create_project_with_settings(s: &Setup, name: &str, settings: ProjectSettings) -> Project {
    s.store
        .create_project(
            ctx(&s.owner, s.clock.now(), None),
            ProjectCreate {
                name: name.to_string(),
                settings: Some(settings),
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

async fn table_count(conn: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(conn)
        .await
        .unwrap()
}

// ── Acceptance regression tests ──────────────────────────────────────

#[tokio::test]
async fn task_without_epic_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let _goal = create_goal(&s, project.id, "G").await;
    let fake_epic = EpicId::generate(s.clock.now());

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            fake_epic,
            TaskCreate {
                title: "Orphan".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let mut conn = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut conn, "tasks").await, 0);
}

#[tokio::test]
async fn wrong_project_epic_fails() {
    let s = setup().await;
    let project_a = create_project(&s, "A").await;
    let goal_a = create_goal(&s, project_a.id, "GA").await;
    let epic_a = create_epic(&s, project_a.id, goal_a.id, "EA").await;

    let project_b = create_project(&s, "B").await;

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project_b.id,
            epic_a.id,
            TaskCreate {
                title: "Cross-project".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let mut conn = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut conn, "tasks").await, 0);
}

#[tokio::test]
async fn epic_is_never_a_task_type_or_claimable() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Epic key does not appear in task_types.
    let types = s
        .store
        .list_task_types(&project.id, &ListParams::default())
        .await
        .unwrap();
    assert!(
        types.items.iter().all(|t| t.key != "epic"),
        "no 'epic' key in the task type registry"
    );

    // Epic is a distinct struct with no claim-related fields.
    assert_eq!(epic.status, EpicStatus::Open);
    assert_eq!(epic.task_counts, Counts::ZERO);
}

#[tokio::test]
async fn changing_defaults_affects_only_future_tasks() {
    let s = setup().await;
    let original_settings = ProjectSettings {
        proposal_gate: false,
        planning_required: true,
        plan_review: ReviewPolicy::Human,
        work_review: ReviewPolicy::Human,
    };
    let project = create_project_with_settings(&s, "P", original_settings).await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let task_before = create_task(&s, project.id, epic.id, "Before").await;
    assert!(task_before.planning_required);
    assert_eq!(task_before.plan_review, ReviewPolicy::Human);
    assert_eq!(task_before.work_review, ReviewPolicy::Human);

    // Update project settings.
    s.clock.advance(TimeDelta::seconds(1));
    let new_settings = ProjectSettings {
        proposal_gate: false,
        planning_required: false,
        plan_review: ReviewPolicy::None,
        work_review: ReviewPolicy::Agent,
    };
    s.store
        .update_project(
            ctx(&s.owner, s.clock.now(), Some(project.revision.value())),
            project.id,
            ProjectPatch {
                settings: Some(new_settings),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Existing task is unchanged.
    let reloaded = s
        .store
        .get_task(&project.id, &task_before.id)
        .await
        .unwrap();
    assert!(reloaded.planning_required);
    assert_eq!(reloaded.plan_review, ReviewPolicy::Human);
    assert_eq!(reloaded.work_review, ReviewPolicy::Human);

    // New task gets new defaults.
    let task_after = create_task(&s, project.id, epic.id, "After").await;
    assert!(!task_after.planning_required);
    assert_eq!(task_after.plan_review, ReviewPolicy::None);
    assert_eq!(task_after.work_review, ReviewPolicy::Agent);
}

#[tokio::test]
async fn agent_cannot_lower_inherited_requirements() {
    let s = setup().await;
    let strict_settings = ProjectSettings {
        proposal_gate: true,
        planning_required: true,
        plan_review: ReviewPolicy::Human,
        work_review: ReviewPolicy::Human,
    };
    let project = create_project_with_settings(&s, "P", strict_settings).await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    // Agent tries to lower plan_review.
    let err = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Lowered".to_string(),
                type_key: "code".to_string(),
                plan_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    // Agent tries to lower planning_required.
    let err = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Lowered".to_string(),
                type_key: "code".to_string(),
                planning_required: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    let mut conn = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut conn, "tasks").await, 0);
}

#[tokio::test]
async fn no_status_patch_is_accepted() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    // TaskPatch has no status or phase fields — verify structurally.
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            TaskPatch {
                title: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.status, task.status);
    assert_eq!(updated.phase, task.phase);
    assert_eq!(updated.title, "Renamed");
}

// ── Epic domain tests ────────────────────────────────────────────────

#[tokio::test]
async fn create_epic_under_valid_goal() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;

    let created = s
        .store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "  Epic One  ".to_string(),
                description: Some("Description".to_string()),
            },
        )
        .await
        .unwrap();
    let epic = created.value;
    assert_eq!(epic.title, "Epic One");
    assert_eq!(epic.description, "Description");
    assert_eq!(epic.revision.value(), 1);
    assert_eq!(epic.project_id, project.id);
    assert_eq!(epic.goal_id, goal.id);
    assert_eq!(epic.status, EpicStatus::Open);
    assert!(!epic.archived);
    assert_eq!(epic.task_counts, Counts::ZERO);
    assert!(epic.block.is_none());
    assert!(epic.archive.is_none());
    assert!(epic.cancellation.is_none());
    assert_eq!(created.events.len(), 1);
}

#[tokio::test]
async fn create_epic_under_archived_project_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;

    sqlx::query(
        "UPDATE projects SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(project.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "Late".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

#[tokio::test]
async fn create_epic_under_archived_goal_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;

    sqlx::query(
        "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(goal.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "Late".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

#[tokio::test]
async fn agent_creates_proposed_epic_when_proposal_gate() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;

    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
        .store
        .create_epic(
            ctx(&agent, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "Agent epic".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(epic.status, EpicStatus::Proposed);
}

#[tokio::test]
async fn update_epic_enforces_revision_and_applies_patch() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Revision conflict.
    let err = s
        .store
        .update_epic(
            ctx(&s.owner, s.clock.now(), Some(9)),
            project.id,
            epic.id,
            TextPatch {
                title: Some("New".to_string()),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::RevisionConflict { .. }));

    // Successful update.
    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            TextPatch {
                title: Some("Renamed".to_string()),
                description: Some("New desc".to_string()),
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.revision.value(), 2);
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.description, "New desc");
    assert_eq!(updated.updated_at, s.clock.now());
}

#[tokio::test]
async fn update_terminal_epic_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Seed cancelled status.
    sqlx::query(
        "UPDATE epics SET status = 'cancelled', cancellation_actor_id = ?1, \
         cancellation_reason = 'not needed', cancellation_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(epic.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .update_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            TextPatch {
                title: Some("Revived".to_string()),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
}

#[tokio::test]
async fn accept_epic_transitions_proposed_to_open() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
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
    assert_eq!(epic.status, EpicStatus::Proposed);

    s.clock.advance(TimeDelta::seconds(1));
    let accepted = s
        .store
        .accept_epic(ctx(&s.owner, s.clock.now(), Some(1)), project.id, epic.id)
        .await
        .unwrap()
        .value;
    assert_eq!(accepted.status, EpicStatus::Open);
    assert_eq!(accepted.revision.value(), 2);
}

#[tokio::test]
async fn accept_non_proposed_epic_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    assert_eq!(epic.status, EpicStatus::Open);

    let err = s
        .store
        .accept_epic(ctx(&s.owner, s.clock.now(), Some(1)), project.id, epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));
}

// ── Task domain tests ────────────────────────────────────────────────

#[tokio::test]
async fn create_task_under_valid_epic() {
    let s = setup().await;
    let settings = ProjectSettings {
        proposal_gate: false,
        planning_required: true,
        plan_review: ReviewPolicy::Agent,
        work_review: ReviewPolicy::Human,
    };
    let project = create_project_with_settings(&s, "P", settings).await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let task = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Task One".to_string(),
                description: Some("Desc".to_string()),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task.title, "Task One");
    assert_eq!(task.description, "Desc");
    assert_eq!(task.revision.value(), 1);
    assert_eq!(task.project_id, project.id);
    assert_eq!(task.epic_id, epic.id);
    assert_eq!(task.type_key, "code");
    assert_eq!(task.status, TaskStatus::Open);
    assert_eq!(task.phase, TaskPhase::Planning);
    assert!(task.planning_required);
    assert_eq!(task.plan_review, ReviewPolicy::Agent);
    assert_eq!(task.work_review, ReviewPolicy::Human);
    assert_eq!(task.attempt_count, 0);
    assert!(!task.archived);
}

#[tokio::test]
async fn create_task_under_terminal_epic_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Seed done status.
    sqlx::query("UPDATE epics SET status = 'done' WHERE id = ?1")
        .bind(epic.id.to_string())
        .execute(s.store.pool())
        .await
        .unwrap();

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

#[tokio::test]
async fn create_task_validates_type_key_exists() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Bad type".to_string(),
                type_key: "nonexistent".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "type_key",
            ..
        }
    ));
}

#[tokio::test]
async fn create_task_rejects_archived_type_key() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    sqlx::query("UPDATE task_types SET archived = 1 WHERE project_id = ?1 AND key = 'design'")
        .bind(project.id.to_string())
        .execute(s.store.pool())
        .await
        .unwrap();

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Archived type".to_string(),
                type_key: "design".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "type_key",
            ..
        }
    ));
}

#[tokio::test]
async fn accept_task_transitions_proposed_to_open() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            planning_required: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Accept the epic first (it's proposed too due to proposal_gate, but human created it).
    // Actually human creates open, so no need to accept.

    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Agent task".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task.status, TaskStatus::Proposed);
    assert_eq!(task.phase, TaskPhase::Planning);

    s.clock.advance(TimeDelta::seconds(1));
    let accepted = s
        .store
        .accept_task(ctx(&s.owner, s.clock.now(), Some(1)), project.id, task.id)
        .await
        .unwrap()
        .value;
    assert_eq!(accepted.status, TaskStatus::Open);
    assert_eq!(accepted.phase, TaskPhase::Planning);
    assert_eq!(accepted.revision.value(), 2);
}

#[tokio::test]
async fn update_task_policy_resets_phase() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: false,
            plan_review: ReviewPolicy::None,
            work_review: ReviewPolicy::None,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.phase, TaskPhase::Execution);

    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                planning_required: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.phase, TaskPhase::Planning);
    assert!(updated.planning_required);
}

#[tokio::test]
async fn update_task_title_retains_phase() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.phase, TaskPhase::Planning);

    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                title: Some("Renamed only".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.phase, TaskPhase::Planning);
    assert_eq!(updated.title, "Renamed only");
}

// ── List and count tests ─────────────────────────────────────────────

#[tokio::test]
async fn list_epics_pages_under_goal() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let e1 = create_epic(&s, project.id, goal.id, "e1").await;
    let e2 = create_epic(&s, project.id, goal.id, "e2").await;
    s.clock.advance(TimeDelta::milliseconds(3));
    let e3 = create_epic(&s, project.id, goal.id, "e3").await;

    let page1 = s
        .store
        .list_epics(
            &project.id,
            &goal.id,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page1.items.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![e1.id, e2.id]
    );
    assert!(page1.next_cursor.is_some());

    let page2 = s
        .store
        .list_epics(
            &project.id,
            &goal.id,
            &ListParams {
                limit: Some(2),
                cursor: page1.next_cursor,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.items[0].id, e3.id);
    assert!(page2.next_cursor.is_none());
}

#[tokio::test]
async fn list_tasks_pages_under_epic() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let t1 = create_task(&s, project.id, epic.id, "t1").await;
    let t2 = create_task(&s, project.id, epic.id, "t2").await;
    s.clock.advance(TimeDelta::milliseconds(3));
    let t3 = create_task(&s, project.id, epic.id, "t3").await;

    let page1 = s
        .store
        .list_tasks(
            &project.id,
            &epic.id,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page1.items.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t1.id, t2.id]
    );

    let page2 = s
        .store
        .list_tasks(
            &project.id,
            &epic.id,
            &ListParams {
                limit: Some(2),
                cursor: page1.next_cursor,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.items[0].id, t3.id);
    assert!(page2.next_cursor.is_none());
}

#[tokio::test]
async fn task_counts_aggregate_statuses() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    // Seed tasks with various statuses directly.
    // cancelled rows need cancellation_actor_id per CHECK constraint.
    let now = format_ts(&s.clock.now());
    let owner_id = s.owner.id.to_string();
    for (i, (status, phase, cancelled, waiver)) in [
        ("done", "complete", false, false),
        ("done", "complete", false, false),
        ("cancelled", "complete", true, false),
        ("cancelled", "complete", true, true),
        ("open", "execution", false, false),
    ]
    .iter()
    .enumerate()
    {
        let id = TaskId::generate(s.clock.now()).to_string();
        sqlx::query(
            "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, epic_id, \
             title, description, type_key, status, phase, planning_required, plan_review, \
             work_review, archived, attempt_count, \
             cancellation_actor_id, cancellation_reason, cancellation_created_at, \
             waiver_actor_id, waiver_reason, waiver_created_at) \
             VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5, '', 'code', ?6, ?7, 0, 'none', 'none', 0, 0, \
             ?8, ?9, ?10, ?11, ?12, ?13)",
        )
        .bind(&id)
        .bind(&now)
        .bind(project.id.to_string())
        .bind(epic.id.to_string())
        .bind(format!("task-{i}"))
        .bind(status)
        .bind(phase)
        .bind(cancelled.then_some(&owner_id))
        .bind(cancelled.then_some("not needed"))
        .bind(cancelled.then_some(now.clone()))
        .bind(waiver.then_some(&owner_id))
        .bind(waiver.then_some("waived"))
        .bind(waiver.then_some(now.clone()))
        .execute(s.store.pool())
        .await
        .unwrap();
    }

    let fetched = s.store.get_epic(&project.id, &epic.id).await.unwrap();
    assert_eq!(
        fetched.task_counts,
        Counts {
            total: 5,
            done: 2,
            cancelled: 2,
            waived: 1,
        }
    );
}

#[tokio::test]
async fn mutations_beneath_archived_epic_rejected() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    sqlx::query(
        "UPDATE epics SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(epic.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

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
    assert!(matches!(err, DomainError::ArchivedScope));
}

// ── Archived-scope bug fixes ─────────────────────────────────────────

#[tokio::test]
async fn update_epic_beneath_archived_goal_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    sqlx::query(
        "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(goal.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .update_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            TextPatch {
                title: Some("Renamed".to_string()),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));

    let mut conn = independent_connection(&s.db_path()).await;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM epics WHERE id = ?1")
        .bind(epic.id.to_string())
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(revision, 1);
}

#[tokio::test]
async fn accept_epic_beneath_archived_goal_fails() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
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
    assert_eq!(epic.status, EpicStatus::Proposed);

    sqlx::query(
        "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(goal.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .accept_epic(ctx(&s.owner, s.clock.now(), Some(1)), project.id, epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

#[tokio::test]
async fn update_task_beneath_archived_epic_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    sqlx::query(
        "UPDATE epics SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(epic.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                title: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));

    let mut conn = independent_connection(&s.db_path()).await;
    let (revision, title): (i64, String) =
        sqlx::query_as("SELECT revision, title FROM tasks WHERE id = ?1")
            .bind(task.id.to_string())
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(revision, 1);
    assert_eq!(title, "T");
}

#[tokio::test]
async fn accept_task_beneath_archived_epic_fails() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Proposed".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task.status, TaskStatus::Proposed);

    sqlx::query(
        "UPDATE epics SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(epic.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .accept_task(ctx(&s.owner, s.clock.now(), Some(1)), project.id, task.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

// ── Cross-entity and permission tests ────────────────────────────────

#[tokio::test]
async fn create_epic_with_goal_from_wrong_project() {
    let s = setup().await;
    let project_a = create_project(&s, "A").await;
    let goal_a = create_goal(&s, project_a.id, "GA").await;
    let project_b = create_project(&s, "B").await;

    let err = s
        .store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project_b.id,
            goal_a.id,
            EpicCreate {
                title: "Cross".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let mut conn = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut conn, "epics").await, 0);
}

#[tokio::test]
async fn accept_epic_by_agent_fails() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
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

    let err = s
        .store
        .accept_epic(ctx(&agent, s.clock.now(), Some(1)), project.id, epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn accept_task_by_agent_fails() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Proposed".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    let err = s
        .store
        .accept_task(ctx(&agent, s.clock.now(), Some(1)), project.id, task.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn accept_non_proposed_task_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.status, TaskStatus::Open);

    let err = s
        .store
        .accept_task(ctx(&s.owner, s.clock.now(), Some(1)), project.id, task.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));
}

#[tokio::test]
async fn update_terminal_task_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    sqlx::query(
        "UPDATE tasks SET status = 'cancelled', phase = 'complete', \
         cancellation_actor_id = ?1, cancellation_reason = 'not needed', \
         cancellation_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(task.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                title: Some("Revived".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));

    let mut conn = independent_connection(&s.db_path()).await;
    let title: String = sqlx::query_scalar("SELECT title FROM tasks WHERE id = ?1")
        .bind(task.id.to_string())
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(title, "T");
}

#[tokio::test]
async fn update_task_agent_cannot_lower_policy() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: true,
            plan_review: ReviewPolicy::Human,
            work_review: ReviewPolicy::Human,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let err = s
        .store
        .update_task(
            ctx(&agent, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                work_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn accepting_task_under_terminal_epic_fails() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "T".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    sqlx::query(
        "UPDATE epics SET status = 'cancelled', cancellation_actor_id = ?1, \
         cancellation_reason = 'done', cancellation_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(epic.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let err = s
        .store
        .accept_task(ctx(&s.owner, s.clock.now(), Some(1)), project.id, task.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
}

#[tokio::test]
async fn accepting_epic_does_not_cascade_to_tasks() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
        .store
        .create_epic(
            ctx(&agent, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "E".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let task1 = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "T1".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let task2 = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "T2".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    s.clock.advance(TimeDelta::seconds(1));
    let accepted = s
        .store
        .accept_epic(ctx(&s.owner, s.clock.now(), Some(1)), project.id, epic.id)
        .await
        .unwrap()
        .value;
    assert_eq!(accepted.status, EpicStatus::Open);

    let t1 = s.store.get_task(&project.id, &task1.id).await.unwrap();
    let t2 = s.store.get_task(&project.id, &task2.id).await.unwrap();
    assert_eq!(t1.status, TaskStatus::Proposed);
    assert_eq!(t2.status, TaskStatus::Proposed);
}

// ── Input validation tests ───────────────────────────────────────────

#[tokio::test]
async fn create_epic_validates_title_and_description() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;

    for title in ["   ", &"x".repeat(201)] {
        let err = s
            .store
            .create_epic(
                ctx(&s.owner, s.clock.now(), None),
                project.id,
                goal.id,
                EpicCreate {
                    title: title.to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Validation { field: "title", .. }),
            "expected title validation for {title:?}"
        );
    }

    let err = s
        .store
        .create_epic(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            goal.id,
            EpicCreate {
                title: "ok".to_string(),
                description: Some("d".repeat(10_001)),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "description",
            ..
        }
    ));
}

#[tokio::test]
async fn create_task_validates_title_and_description() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "   ".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "title", .. }
    ));

    let err = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "ok".to_string(),
                type_key: "code".to_string(),
                description: Some("d".repeat(10_001)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "description",
            ..
        }
    ));
}

#[tokio::test]
async fn update_epic_empty_patch_rejected() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let err = s
        .store
        .update_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic.id,
            TextPatch::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "patch", .. }
    ));
}

#[tokio::test]
async fn update_task_empty_patch_rejected() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    let err = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "patch", .. }
    ));
}

#[tokio::test]
async fn update_task_rejects_archived_type_key_on_update() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    sqlx::query("UPDATE task_types SET archived = 1 WHERE project_id = ?1 AND key = 'design'")
        .bind(project.id.to_string())
        .execute(s.store.pool())
        .await
        .unwrap();

    let err = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                type_key: Some("design".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "type_key",
            ..
        }
    ));
}

// ── Positive / edge case tests ───────────────────────────────────────

#[tokio::test]
async fn agent_creates_task_with_default_policy() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: false,
            planning_required: true,
            plan_review: ReviewPolicy::Agent,
            work_review: ReviewPolicy::Agent,
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Agent task".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task.status, TaskStatus::Open);
    assert!(task.planning_required);
    assert_eq!(task.plan_review, ReviewPolicy::Agent);
    assert_eq!(task.work_review, ReviewPolicy::Agent);
    assert_eq!(task.phase, TaskPhase::Planning);
}

#[tokio::test]
async fn create_task_under_proposed_epic_succeeds() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            proposal_gate: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "bot");
    register(&s.store, &agent).await;

    let epic = s
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
    assert_eq!(epic.status, EpicStatus::Proposed);

    let task = s
        .store
        .create_task(
            ctx(&agent, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Under proposed".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task.epic_id, epic.id);
    assert_eq!(task.status, TaskStatus::Proposed);
}

#[tokio::test]
async fn update_task_type_key_change() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: true,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.type_key, "code");
    assert_eq!(task.phase, TaskPhase::Planning);

    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                type_key: Some("research".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.type_key, "research");
    assert_eq!(updated.phase, TaskPhase::Planning);
}

#[tokio::test]
async fn update_task_description_resets_phase() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: false,
            plan_review: ReviewPolicy::None,
            work_review: ReviewPolicy::None,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.phase, TaskPhase::Execution);

    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                description: Some("New description".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.description, "New description");
    assert_eq!(updated.phase, TaskPhase::Execution);
}

#[tokio::test]
async fn update_task_same_policy_does_not_reset_phase() {
    let s = setup().await;
    let project = create_project_with_settings(
        &s,
        "P",
        ProjectSettings {
            planning_required: true,
            plan_review: ReviewPolicy::Human,
            work_review: ReviewPolicy::Human,
            ..Default::default()
        },
    )
    .await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;
    assert_eq!(task.phase, TaskPhase::Planning);

    s.clock.advance(TimeDelta::seconds(1));
    let updated = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            TaskPatch {
                planning_required: Some(true),
                plan_review: Some(ReviewPolicy::Human),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.phase, TaskPhase::Planning);
    assert!(updated.planning_required);
}

#[tokio::test]
async fn task_counts_include_archived_descendants() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let _live = create_task(&s, project.id, epic.id, "live").await;
    let archived = create_task(&s, project.id, epic.id, "archived").await;

    sqlx::query(
        "UPDATE tasks SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2, \
         status = 'done', phase = 'complete' WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(archived.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();

    let fetched = s.store.get_epic(&project.id, &epic.id).await.unwrap();
    assert_eq!(fetched.task_counts.total, 2);
    assert_eq!(fetched.task_counts.done, 1);
}

#[tokio::test]
async fn list_epics_empty_goal_returns_empty_page() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;

    let page = s
        .store
        .list_epics(&project.id, &goal.id, &ListParams::default())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn list_tasks_nonexistent_epic_returns_not_found() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let fake_epic = EpicId::generate(s.clock.now());

    let err = s
        .store
        .list_tasks(&project.id, &fake_epic, &ListParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn get_epic_nonexistent_returns_not_found() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let fake_epic = EpicId::generate(s.clock.now());

    let err = s.store.get_epic(&project.id, &fake_epic).await.unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn get_task_nonexistent_returns_not_found() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let fake_task = TaskId::generate(s.clock.now());

    let err = s.store.get_task(&project.id, &fake_task).await.unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}
