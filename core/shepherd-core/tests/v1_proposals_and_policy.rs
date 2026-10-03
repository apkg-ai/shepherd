use std::sync::Arc;

use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, EpicCreate, EpicStatus, GoalCreate, ProjectCreate,
    ProjectPatch, ProjectSettings, ReviewPolicy, TaskCreate, TaskPatch, TaskStatus, TaskTypeCreate,
    TaskTypeId, TaskTypePatch, TestClock,
};
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, open, testing};
use uuid::Uuid;

struct Fixture {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
    owner: Actor,
}

fn make_actor(clock: &TestClock, kind: ActorKind, label: &str) -> Actor {
    Actor {
        id: ActorId::generate(clock.now()),
        kind,
        label: label.to_string(),
        revoked: false,
        created_at: clock.now(),
    }
}

async fn register(store: &Store, actor: &Actor) {
    let inserted = actor.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
        .await
        .unwrap();
}

fn ctx(actor: &Actor, clock: &TestClock, expected_revision: Option<i64>) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(clock.now()),
        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        expected_revision,
        now: clock.now(),
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = open(testing::store_options(
        dir.path(),
        "shepherd.db",
        clock.clone(),
    ))
    .await
    .unwrap();
    let owner = make_actor(&clock, ActorKind::Human, "owner");
    register(&store, &owner).await;
    Fixture {
        _dir: dir,
        clock,
        store,
        owner,
    }
}

#[tokio::test]
async fn proposal_gate_defaults_on_and_owner_disables() {
    let f = fixture().await;
    let project = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert!(project.settings.proposal_gate);

    let goal = f
        .store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
            project.id,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;

    let agent = make_actor(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    let proposed_epic = f
        .store
        .create_epic(
            ctx(&agent, &f.clock, None),
            project.id,
            goal.id,
            EpicCreate {
                title: "E1".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(proposed_epic.status, EpicStatus::Proposed);

    let proposed_task = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            proposed_epic.id,
            TaskCreate {
                title: "T1".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(proposed_task.status, TaskStatus::Proposed);

    // Owner disables proposal gate.
    f.store
        .update_project(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            ProjectPatch {
                settings: Some(ProjectSettings {
                    proposal_gate: false,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let open_epic = f
        .store
        .create_epic(
            ctx(&agent, &f.clock, None),
            project.id,
            goal.id,
            EpicCreate {
                title: "E2".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(open_epic.status, EpicStatus::Open);

    let open_task = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            open_epic.id,
            TaskCreate {
                title: "T2".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(open_task.status, TaskStatus::Open);

    // Persisted state: old items unchanged.
    let (epic_rev, epic_status): (i64, String) =
        sqlx::query_as("SELECT revision, status FROM epics WHERE id = ?1")
            .bind(proposed_epic.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(epic_rev, 1);
    assert_eq!(epic_status, "proposed");

    let (task_rev, task_status): (i64, String) =
        sqlx::query_as("SELECT revision, status FROM tasks WHERE id = ?1")
            .bind(proposed_task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(task_rev, 1);
    assert_eq!(task_status, "proposed");
}

#[tokio::test]
async fn accepting_epic_leaves_proposed_tasks_proposed() {
    let f = fixture().await;
    let project = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "P".to_string(),
                settings: Some(ProjectSettings {
                    proposal_gate: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let goal = f
        .store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
            project.id,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;

    let agent = make_actor(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    let epic = f
        .store
        .create_epic(
            ctx(&agent, &f.clock, None),
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
    assert_eq!(epic.status, EpicStatus::Proposed);

    let task_a = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "A".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let task_b = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "B".to_string(),
                type_key: "research".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(task_a.status, TaskStatus::Proposed);
    assert_eq!(task_b.status, TaskStatus::Proposed);

    // Accept the epic; tasks must remain proposed.
    let accepted = f
        .store
        .accept_epic(ctx(&f.owner, &f.clock, Some(1)), project.id, epic.id)
        .await
        .unwrap()
        .value;
    assert_eq!(accepted.status, EpicStatus::Open);
    assert_eq!(accepted.revision.value(), 2);

    for task_id in [&task_a.id, &task_b.id] {
        let (revision, status): (i64, String) =
            sqlx::query_as("SELECT revision, status FROM tasks WHERE id = ?1")
                .bind(task_id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(revision, 1, "task revision must not change on epic accept");
        assert_eq!(
            status, "proposed",
            "task must stay proposed after epic accept"
        );
    }
}

#[tokio::test]
async fn policy_downgrade_fails_for_agents_human_edit_fails_during_active_work() {
    let f = fixture().await;
    let project = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "P".to_string(),
                settings: Some(ProjectSettings {
                    proposal_gate: false,
                    planning_required: true,
                    plan_review: ReviewPolicy::Human,
                    work_review: ReviewPolicy::Human,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let goal = f
        .store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
            project.id,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let epic = f
        .store
        .create_epic(
            ctx(&f.owner, &f.clock, None),
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

    let agent = make_actor(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    // Snapshot before any failed commands to prove none of them leak events.
    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(f.store.pool())
        .await
        .unwrap();

    // Agent cannot lower planning_required on create.
    let err = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Low".to_string(),
                type_key: "code".to_string(),
                planning_required: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    // Agent cannot lower plan_review on create.
    let err = f
        .store
        .create_task(
            ctx(&agent, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Low".to_string(),
                type_key: "code".to_string(),
                plan_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    // Owner creates task with inherited policy.
    let task = f
        .store
        .create_task(
            ctx(&f.owner, &f.clock, None),
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
    assert!(task.planning_required);
    assert_eq!(task.plan_review, ReviewPolicy::Human);

    // Agent cannot update policy fields (LowerRequirements capability).
    let err = f
        .store
        .update_task(
            ctx(&agent, &f.clock, Some(1)),
            project.id,
            task.id,
            TaskPatch {
                plan_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    // Seed an active claim; owner edits are blocked during active work.
    let now_str = format_ts(&f.clock.now());
    testing::seed_claim(
        f.store.pool(),
        task.id,
        f.owner.id,
        "execute",
        "active",
        &now_str,
        "2099-01-01T00:00:00.000Z",
    )
    .await;

    let err = f
        .store
        .update_task(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            task.id,
            TaskPatch {
                title: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));

    // No revision bump; only the owner's successful create_task added events.
    let task_rev: i64 = sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
        .bind(task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    assert_eq!(task_rev, 1);

    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    // Only the owner's successful create_task (1 event) should have fired.
    assert_eq!(events_after, events_before + 1);
}

#[tokio::test]
async fn archived_type_cannot_be_assigned_but_existing_task_renders_label() {
    let f = fixture().await;
    let project = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let goal = f
        .store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
            project.id,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let epic = f
        .store
        .create_epic(
            ctx(&f.owner, &f.clock, None),
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

    let ops_type = f
        .store
        .create_task_type(
            ctx(&f.owner, &f.clock, None),
            project.id,
            TaskTypeCreate {
                key: "ops".to_string(),
                label: "Operations".to_string(),
            },
        )
        .await
        .unwrap()
        .value;

    let existing = f
        .store
        .create_task(
            ctx(&f.owner, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Existing".to_string(),
                type_key: "ops".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    let other = f
        .store
        .create_task(
            ctx(&f.owner, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "Other".to_string(),
                type_key: "code".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    // The archived flag is one-way: clearing it is rejected.
    let err = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            ops_type.id,
            TaskTypePatch {
                archived: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "archived",
            ..
        }
    ));

    // Agents cannot archive types (AdministerProject is owner-only).
    let agent = make_actor(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;
    let err = f
        .store
        .update_task_type(
            ctx(&agent, &f.clock, Some(1)),
            project.id,
            ops_type.id,
            TaskTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    // Archive the ops type via the contract patch field.
    let archived = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            ops_type.id,
            TaskTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert!(archived.archived);
    assert_eq!(archived.revision.value(), 2);
    assert_eq!(archived.key, "ops");
    assert_eq!(archived.label, "Operations");

    // Double-archive is rejected.
    let err = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(2)),
            project.id,
            ops_type.id,
            TaskTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));

    // Archiving a type under an archived project is rejected.
    let code_type: TaskTypeId =
        sqlx::query_scalar("SELECT id FROM task_types WHERE project_id = ?1 AND key = 'code'")
            .bind(project.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .map(|id: String| id.parse().unwrap())
            .unwrap();
    sqlx::query(
        "UPDATE projects SET archived = 1, archive_actor_id = ?1, \
         archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
    )
    .bind(f.owner.id.to_string())
    .bind(format_ts(&f.clock.now()))
    .bind(project.id.to_string())
    .execute(f.store.pool())
    .await
    .unwrap();
    let err = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            code_type,
            TaskTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    // Restore project for remaining assertions.
    sqlx::query(
        "UPDATE projects SET archived = 0, archive_actor_id = NULL, \
         archive_reason = NULL, archive_created_at = NULL WHERE id = ?1",
    )
    .bind(project.id.to_string())
    .execute(f.store.pool())
    .await
    .unwrap();

    // New task with archived type is rejected.
    let err = f
        .store
        .create_task(
            ctx(&f.owner, &f.clock, None),
            project.id,
            epic.id,
            TaskCreate {
                title: "New".to_string(),
                type_key: "ops".to_string(),
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

    // Updating another task's type to the archived key is rejected.
    let err = f
        .store
        .update_task(
            ctx(&f.owner, &f.clock, Some(1)),
            project.id,
            other.id,
            TaskPatch {
                type_key: Some("ops".to_string()),
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

    // Existing task retains its type_key.
    let task_key: String = sqlx::query_scalar("SELECT type_key FROM tasks WHERE id = ?1")
        .bind(existing.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    assert_eq!(task_key, "ops");

    // Task type row is still accessible with archived flag and preserved label.
    let (tt_archived, tt_label, tt_key): (i64, String, String) =
        sqlx::query_as("SELECT archived, label, key FROM task_types WHERE id = ?1")
            .bind(ops_type.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(tt_archived, 1);
    assert_eq!(tt_label, "Operations");
    assert_eq!(tt_key, "ops");
}
