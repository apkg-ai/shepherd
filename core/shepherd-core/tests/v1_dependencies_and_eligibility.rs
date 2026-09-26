use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, Dependency, DependencyCreate, Epic, EpicCreate,
    EpicId, Goal, GoalCreate, GoalId, Project, ProjectCreate, ProjectId, Task, TaskCreate, TaskId,
    TestClock,
};
use shepherd_core::queries::{ListParams, WorkFilters, WorkPhase};
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, open, testing};
use shepherd_core::workflow::eligibility::GateCode;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
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

async fn create_planning_task(
    s: &Setup,
    actor: &Actor,
    project: ProjectId,
    epic: EpicId,
    title: &str,
) -> Task {
    s.clock.advance(chrono::TimeDelta::milliseconds(2));
    s.store
        .create_task(
            ctx(actor, s.clock.now(), None),
            project,
            epic,
            TaskCreate {
                title: title.to_string(),
                type_key: "code".to_string(),
                planning_required: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

async fn link_tasks(
    s: &Setup,
    project: ProjectId,
    dependent: TaskId,
    prerequisite: TaskId,
) -> Dependency {
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

// Direct-SQL fixture: completion commands land in step 006.
async fn mark_task_done(s: &Setup, task: TaskId) {
    sqlx::query("UPDATE tasks SET status = 'done', phase = 'complete' WHERE id = ?1")
        .bind(task.to_string())
        .execute(s.store.pool())
        .await
        .unwrap();
}

async fn work_ids(s: &Setup, project: &ProjectId, phase: WorkPhase) -> Vec<TaskId> {
    s.store
        .list_work(
            &s.owner,
            project,
            phase,
            &WorkFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap()
        .items
        .iter()
        .map(|item| item.task.id)
        .collect()
}

#[tokio::test]
async fn cross_goal_epic_link_is_scope_mismatch() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal_a = create_goal(&s, project.id, "A").await;
    let goal_b = create_goal(&s, project.id, "B").await;
    let epic_a = create_epic(&s, project.id, goal_a.id, "in A").await;
    let epic_b = create_epic(&s, project.id, goal_b.id, "in B").await;
    let mut conn = independent_connection(&s.db_path()).await;
    let events_before = table_count(&mut conn, "events").await;

    let err = s
        .store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Epic {
                dependent_id: epic_a.id,
                prerequisite_id: epic_b.id,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ScopeMismatch(_)));
    assert_eq!(err.code(), "scope_mismatch");

    // The failed mutation left revisions and history untouched.
    assert_eq!(table_count(&mut conn, "epic_dependencies").await, 0);
    assert_eq!(
        revision_of(&mut conn, "epics", epic_a.id.as_uuid()).await,
        1
    );
    assert_eq!(
        revision_of(&mut conn, "epics", epic_b.id.as_uuid()).await,
        1
    );
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn cross_epic_task_link_is_scope_mismatch() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic_a = create_epic(&s, project.id, goal.id, "A").await;
    let epic_b = create_epic(&s, project.id, goal.id, "B").await;
    let task_a = create_task(&s, project.id, epic_a.id, "a").await;
    let task_b = create_task(&s, project.id, epic_b.id, "b").await;
    let mut conn = independent_connection(&s.db_path()).await;
    let events_before = table_count(&mut conn, "events").await;

    let err = s
        .store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Task {
                dependent_id: task_a.id,
                prerequisite_id: task_b.id,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ScopeMismatch(_)));

    assert_eq!(table_count(&mut conn, "task_dependencies").await, 0);
    assert_eq!(
        revision_of(&mut conn, "tasks", task_a.id.as_uuid()).await,
        1
    );
    assert_eq!(
        revision_of(&mut conn, "tasks", task_b.id.as_uuid()).await,
        1
    );
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn valid_branch_and_join_passes() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let a = create_task(&s, project.id, epic.id, "a").await;
    let b = create_task(&s, project.id, epic.id, "b").await;
    let c = create_task(&s, project.id, epic.id, "c").await;
    let d = create_task(&s, project.id, epic.id, "d").await;
    // Diamond: b and c branch from a; d joins b and c.
    link_tasks(&s, project.id, b.id, a.id).await;
    link_tasks(&s, project.id, c.id, a.id).await;
    link_tasks(&s, project.id, d.id, b.id).await;
    link_tasks(&s, project.id, d.id, c.id).await;

    let mut conn = independent_connection(&s.db_path()).await;
    assert_eq!(table_count(&mut conn, "task_dependencies").await, 4);
    // Every endpoint took part in exactly two links: two revision bumps each.
    for task in [&a, &b, &c, &d] {
        assert_eq!(revision_of(&mut conn, "tasks", task.id.as_uuid()).await, 3);
    }
    let affected: Vec<String> = sqlx::query_scalar(
        "SELECT affected_ids FROM events WHERE type = 'dependency.changed' ORDER BY id",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(affected.len(), 4);
    let first: Vec<String> = serde_json::from_str(&affected[0]).unwrap();
    assert_eq!(
        first,
        vec![b.id.to_string(), a.id.to_string(), epic.id.to_string()]
    );

    // The same branch/join shape passes at the epic level.
    let e1 = create_epic(&s, project.id, goal.id, "e1").await;
    let e2 = create_epic(&s, project.id, goal.id, "e2").await;
    let e3 = create_epic(&s, project.id, goal.id, "e3").await;
    let e4 = create_epic(&s, project.id, goal.id, "e4").await;
    link_epics(&s, project.id, e2.id, e1.id).await;
    link_epics(&s, project.id, e3.id, e1.id).await;
    link_epics(&s, project.id, e4.id, e2.id).await;
    link_epics(&s, project.id, e4.id, e3.id).await;
    assert_eq!(table_count(&mut conn, "epic_dependencies").await, 4);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn candidate_with_zero_prerequisites_is_eligible() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let candidate = create_task(&s, project.id, epic.id, "candidate").await;

    let page = s
        .store
        .list_work(
            &s.owner,
            &project.id,
            WorkPhase::Execute,
            &WorkFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let item = &page.items[0];
    assert_eq!(item.task.id, candidate.id);
    assert!(item.eligibility.can_execute);
    assert!(item.eligibility.reasons.is_empty());
}

#[tokio::test]
async fn candidate_waits_until_every_prerequisite_done() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let waiting = create_task(&s, project.id, epic.id, "waiting").await;
    let p1 = create_task(&s, project.id, epic.id, "p1").await;
    let p2 = create_task(&s, project.id, epic.id, "p2").await;
    let p3 = create_task(&s, project.id, epic.id, "p3").await;
    for prerequisite in [&p1, &p2] {
        link_tasks(&s, project.id, waiting.id, prerequisite.id).await;
    }
    let p3_link = link_tasks(&s, project.id, waiting.id, p3.id).await;

    assert!(
        !work_ids(&s, &project.id, WorkPhase::Execute)
            .await
            .contains(&waiting.id)
    );
    let graph = s
        .store
        .get_epic_graph(&s.owner, &project.id, &epic.id)
        .await
        .unwrap();
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == waiting.id.as_uuid())
        .unwrap();
    assert!(!node.eligibility.can_execute);
    let mut blocking: Vec<Uuid> = node
        .eligibility
        .reasons
        .iter()
        .filter(|reason| reason.code == GateCode::TaskPrerequisite)
        .map(|reason| reason.resource_id)
        .collect();
    let mut expected = vec![p1.id.as_uuid(), p2.id.as_uuid(), p3.id.as_uuid()];
    blocking.sort();
    expected.sort();
    assert_eq!(blocking, expected);

    // One prerequisite done: still waiting.
    mark_task_done(&s, p1.id).await;
    assert!(
        !work_ids(&s, &project.id, WorkPhase::Execute)
            .await
            .contains(&waiting.id)
    );

    // Cancelled — even waived — prerequisites remain unmet (plan/04).
    // Direct-SQL fixture: cancellation and waiver commands land in step 006.
    mark_task_done(&s, p2.id).await;
    sqlx::query(
        "UPDATE tasks SET status = 'cancelled', phase = 'complete', \
         cancellation_actor_id = ?1, cancellation_reason = 'dropped', \
         cancellation_created_at = ?2, waiver_actor_id = ?1, waiver_reason = 'excluded', \
         waiver_created_at = ?2 WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(p3.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();
    assert!(
        !work_ids(&s, &project.id, WorkPhase::Execute)
            .await
            .contains(&waiting.id)
    );

    // Cancellation is terminal (plan/03): no future command can flip a
    // cancelled prerequisite to done, so the owner removes the obsolete link.
    s.store
        .delete_dependency(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            p3_link.id,
        )
        .await
        .unwrap();
    assert_eq!(
        work_ids(&s, &project.id, WorkPhase::Execute).await,
        vec![waiting.id]
    );
}

#[tokio::test]
async fn early_planning_ignores_dependency_waits_but_not_proposal_or_manual_blocks() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "agent");
    register(&s.store, &agent).await;
    // Default settings keep the proposal gate on: agent creations are proposed.
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;

    let planning = create_planning_task(&s, &s.owner, project.id, epic.id, "planning").await;
    let prerequisite = create_task(&s, project.id, epic.id, "prerequisite").await;
    link_tasks(&s, project.id, planning.id, prerequisite.id).await;
    let blocked = create_planning_task(&s, &s.owner, project.id, epic.id, "blocked").await;
    // Direct-SQL fixture: block commands land in step 006.
    sqlx::query(
        "UPDATE tasks SET block_actor_id = ?1, block_reason = 'hold', block_created_at = ?2 \
         WHERE id = ?3",
    )
    .bind(s.owner.id.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(blocked.id.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();
    let proposed = create_planning_task(&s, &agent, project.id, epic.id, "proposed").await;

    // Dependency waits do not gate planning; proposal and manual blocks do.
    assert_eq!(
        work_ids(&s, &project.id, WorkPhase::Plan).await,
        vec![planning.id]
    );
    let page = s
        .store
        .list_work(
            &s.owner,
            &project.id,
            WorkPhase::Plan,
            &WorkFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap();
    let item = &page.items[0];
    assert!(item.eligibility.can_plan);
    assert!(!item.eligibility.can_execute);
    let codes: Vec<GateCode> = item.eligibility.reasons.iter().map(|r| r.code).collect();
    assert!(codes.contains(&GateCode::TaskPrerequisite));
    assert!(codes.contains(&GateCode::PlanRequired));

    let graph = s
        .store
        .get_epic_graph(&s.owner, &project.id, &epic.id)
        .await
        .unwrap();
    let node = |id: TaskId| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id.as_uuid())
            .unwrap()
    };
    let blocked_node = node(blocked.id);
    assert!(!blocked_node.eligibility.can_plan);
    assert_eq!(
        blocked_node.eligibility.reasons[0].code,
        GateCode::ExplicitBlock
    );
    let proposed_node = node(proposed.id);
    assert!(!proposed_node.eligibility.can_plan);
    assert_eq!(
        proposed_node.eligibility.reasons[0].code,
        GateCode::ProposalRequired
    );
}

#[tokio::test]
async fn cycle_race_yields_at_most_one_accepted_edge() {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    // Independent pools on one file-backed database exercise real serialization.
    let store_a = Arc::new(open_store(&dir, clock.clone()).await);
    let store_b = Arc::new(open_store(&dir, clock.clone()).await);
    let owner = actor(clock.now(), ActorKind::Human, "owner");
    register(&store_a, &owner).await;
    let project = store_a
        .create_project(
            ctx(&owner, clock.now(), None),
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    let goal = store_a
        .create_goal(
            ctx(&owner, clock.now(), None),
            project.id,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let epic = store_a
        .create_epic(
            ctx(&owner, clock.now(), None),
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
    let mut tasks = Vec::new();
    for title in ["t1", "t2"] {
        clock.advance(chrono::TimeDelta::milliseconds(2));
        tasks.push(
            store_a
                .create_task(
                    ctx(&owner, clock.now(), None),
                    project.id,
                    epic.id,
                    TaskCreate {
                        title: title.to_string(),
                        type_key: "code".to_string(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value,
        );
    }
    let (t1, t2) = (tasks[0].id, tasks[1].id);

    let mut handles = Vec::new();
    for (store, dependent, prerequisite) in [(store_a.clone(), t1, t2), (store_b.clone(), t2, t1)] {
        let owner = owner.clone();
        let clock = clock.clone();
        let project_id = project.id;
        handles.push(tokio::spawn(async move {
            store
                .create_dependency(
                    ctx(&owner, clock.now(), None),
                    project_id,
                    DependencyCreate::Task {
                        dependent_id: dependent,
                        prerequisite_id: prerequisite,
                    },
                )
                .await
        }));
    }
    let mut accepted = 0;
    let mut cycles = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => accepted += 1,
            Err(DomainError::DependencyCycle) => cycles += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!(accepted, 1, "exactly one edge wins the race");
    assert_eq!(cycles, 1, "the loser observes the committed edge");

    let mut conn = independent_connection(&dir.path().join("shepherd.db")).await;
    assert_eq!(table_count(&mut conn, "task_dependencies").await, 1);
    let dependency_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE type = 'dependency.changed'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(dependency_events, 1);
    // Only the winner bumped the endpoints.
    assert_eq!(revision_of(&mut conn, "tasks", t1.as_uuid()).await, 2);
    assert_eq!(revision_of(&mut conn, "tasks", t2.as_uuid()).await, 2);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn failed_removal_leaves_revision_and_history_unchanged() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "agent");
    register(&s.store, &agent).await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let a = create_task(&s, project.id, epic.id, "a").await;
    let b = create_task(&s, project.id, epic.id, "b").await;
    let created = s
        .store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Task {
                dependent_id: a.id,
                prerequisite_id: b.id,
            },
        )
        .await
        .unwrap()
        .value;

    let mut conn = independent_connection(&s.db_path()).await;
    let events_before = table_count(&mut conn, "events").await;
    // Agents never remove gates; stale revisions conflict; both leave no trace.
    let err = s
        .store
        .delete_dependency(ctx(&agent, s.clock.now(), Some(1)), project.id, created.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = s
        .store
        .delete_dependency(
            ctx(&s.owner, s.clock.now(), Some(9)),
            project.id,
            created.id,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::RevisionConflict { .. }));

    assert_eq!(table_count(&mut conn, "task_dependencies").await, 1);
    assert_eq!(revision_of(&mut conn, "tasks", a.id.as_uuid()).await, 2);
    assert_eq!(revision_of(&mut conn, "tasks", b.id.as_uuid()).await, 2);
    assert_eq!(table_count(&mut conn, "events").await, events_before);

    // The owner removes the link with the matching revision.
    s.store
        .delete_dependency(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            created.id,
        )
        .await
        .unwrap();
    assert_eq!(table_count(&mut conn, "task_dependencies").await, 0);
    conn.close().await.unwrap();
}
