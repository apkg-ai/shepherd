pub(super) use std::sync::Arc;

pub(super) use uuid::Uuid;

use super::*;
pub(super) use crate::model::{Actor, ActorKind, Clock, DependencyCreate, ReviewPolicy, TestClock};
pub(super) use crate::storage::open;
pub(super) use crate::storage::rows::insert_actor;
pub(super) use crate::storage::testing::{store_options, test_clock};

pub(super) struct Fixture {
    pub(super) _dir: tempfile::TempDir,
    pub(super) clock: Arc<TestClock>,
    pub(super) store: Store,
    pub(super) owner: Actor,
}

pub(super) fn person(clock: &TestClock, kind: ActorKind, label: &str) -> Actor {
    Actor {
        id: crate::model::ActorId::generate(clock.now()),
        kind,
        label: label.to_string(),
        revoked: false,
        created_at: clock.now(),
    }
}

pub(super) async fn register(store: &Store, actor: &Actor) {
    let inserted = actor.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
        .await
        .unwrap();
}

pub(super) async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = test_clock();
    let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
        .await
        .unwrap();
    let owner = person(&clock, ActorKind::Human, "owner");
    register(&store, &owner).await;
    Fixture {
        _dir: dir,
        clock,
        store,
        owner,
    }
}

pub(super) fn ctx(
    actor: &Actor,
    clock: &TestClock,
    expected_revision: Option<i64>,
) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: crate::model::CommandId::generate(clock.now()),
        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        request_hash: Some("test-request".into()),
        expected_revision,
        now: clock.now(),
    }
}

pub(super) async fn project(f: &Fixture, name: &str) -> Project {
    f.store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

pub(super) async fn goal(f: &Fixture, project: ProjectId, title: &str) -> Goal {
    f.store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
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

pub(super) async fn epic(f: &Fixture, project: ProjectId, goal: GoalId, title: &str) -> Epic {
    f.store
        .create_epic(
            ctx(&f.owner, &f.clock, None),
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

pub(super) async fn task(f: &Fixture, project: ProjectId, epic: EpicId, title: &str) -> Task {
    f.store
        .create_task(
            ctx(&f.owner, &f.clock, None),
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

pub(super) async fn seed_claim_row(f: &Fixture, task: TaskId) -> String {
    crate::storage::testing::seed_claim(
        f.store.pool(),
        task,
        f.owner.id,
        "execute",
        "active",
        &format_ts(&f.clock.now()),
        &format_ts(&(f.clock.now() + chrono::Duration::minutes(30))),
    )
    .await
}

pub(super) async fn seed_submission_row(f: &Fixture, task: TaskId) -> String {
    crate::storage::testing::seed_submission(
        f.store.pool(),
        task,
        f.owner.id,
        "pending",
        &format_ts(&f.clock.now()),
    )
    .await
}

/// (type, resource_id, resource_revision, action, reason) per emitted event.
pub(super) async fn event_rows(
    f: &Fixture,
    ids: &[i64],
) -> Vec<(String, String, i64, String, String)> {
    let mut rows = Vec::new();
    for id in ids {
        rows.push(
            sqlx::query_as(
                "SELECT type, resource_id, resource_revision, action, reason \
                 FROM events WHERE id = ?1",
            )
            .bind(id)
            .fetch_one(f.store.pool())
            .await
            .unwrap(),
        );
    }
    rows
}

pub(super) async fn epic_state(f: &Fixture, epic: EpicId) -> (String, i64) {
    sqlx::query_as("SELECT status, revision FROM epics WHERE id = ?1")
        .bind(epic.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap()
}

pub(super) async fn task_state(f: &Fixture, task: TaskId) -> (String, String, i64) {
    sqlx::query_as("SELECT status, phase, revision FROM tasks WHERE id = ?1")
        .bind(task.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap()
}

pub(super) async fn claim_state(f: &Fixture, claim: &str) -> (String, String) {
    sqlx::query_as("SELECT status, close_reason FROM claims WHERE id = ?1")
        .bind(claim)
        .fetch_one(f.store.pool())
        .await
        .unwrap()
}

pub(super) struct LifecycleScope {
    pub(super) project: ProjectId,
    pub(super) epic: Epic,
    pub(super) task: Task,
}

pub(super) async fn lifecycle_scope(f: &Fixture) -> LifecycleScope {
    let p = project(f, "P").await;
    let g = goal(f, p.id, "G").await;
    let e = epic(f, p.id, g.id, "E").await;
    let t = task(f, p.id, e.id, "T").await;
    LifecycleScope {
        project: p.id,
        epic: e,
        task: t,
    }
}
