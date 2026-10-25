// Core surface added for step 015 REST integration: commit notifications,
// the task-sessions list, and the wire task/epic views.

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, EpicCreate, EpicId, GoalCreate, GoalId,
    ProjectCreate, ProjectId, TaskCreate, TaskId, TestClock,
};
use shepherd_core::queries::ListParams;
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, open, testing};
use uuid::Uuid;

fn actor(now: DateTime<Utc>, kind: ActorKind, label: &str) -> Actor {
    Actor {
        id: ActorId::generate(now),
        kind,
        label: label.to_string(),
        revoked: false,
        created_at: now,
    }
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

fn params(limit: Option<u32>, cursor: Option<String>) -> ListParams {
    ListParams {
        limit,
        cursor,
        include_archived: false,
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
    owner: Actor,
    project: ProjectId,
    goal: GoalId,
    epic: EpicId,
    task_a: TaskId,
    task_b: TaskId,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = open(testing::store_options(
        dir.path(),
        "shepherd.db",
        clock.clone(),
    ))
    .await
    .unwrap();
    let owner = actor(clock.now(), ActorKind::Human, "owner");
    let registered = owner.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
        .await
        .unwrap();
    let project = store
        .create_project(
            ctx(&owner, clock.now(), None),
            ProjectCreate {
                name: "P".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let goal = store
        .create_goal(
            ctx(&owner, clock.now(), None),
            project,
            GoalCreate {
                title: "G".into(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let epic = store
        .create_epic(
            ctx(&owner, clock.now(), None),
            project,
            goal,
            EpicCreate {
                title: "E".into(),
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
                    ctx(&owner, clock.now(), None),
                    project,
                    epic,
                    TaskCreate {
                        title: title.into(),
                        description: None,
                        type_key: "code".into(),
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
    Setup {
        _dir: dir,
        clock,
        store,
        owner,
        project,
        goal,
        epic,
        task_a: tasks[0],
        task_b: tasks[1],
    }
}

#[tokio::test]
async fn commits_notify_subscribers_with_the_last_event_id() {
    let s = setup().await;
    let mut rx = s.store.subscribe_events();

    // Plain domain transaction.
    s.store
        .create_goal(
            ctx(&s.owner, s.clock.now(), None),
            s.project,
            GoalCreate {
                title: "G2".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    let after_goal = rx.recv().await.unwrap();
    assert_eq!(after_goal, s.store.event_high_water().await.unwrap());

    // Idempotent transaction path notifies too.
    testing::create_task_document(
        &s.store,
        &s.owner,
        s.project,
        s.task_a,
        shepherd_core::model::DocumentKind::Note,
    )
    .await;
    let after_document = rx.recv().await.unwrap();
    assert_eq!(after_document, s.store.event_high_water().await.unwrap());
    assert!(after_document > after_goal);
}

#[tokio::test]
async fn notifications_without_subscribers_do_not_fail_commits() {
    let s = setup().await;
    // No subscriber exists; the commit must still succeed and persist.
    let goal = s
        .store
        .create_goal(
            ctx(&s.owner, s.clock.now(), None),
            s.project,
            GoalCreate {
                title: "G2".into(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let mut rx = s.store.subscribe_events();
    assert!(
        rx.try_recv().is_err(),
        "no notification is buffered for a late subscriber"
    );
    let listed = s
        .store
        .list_goals(&s.project, &params(None, None))
        .await
        .unwrap();
    assert!(listed.items.iter().any(|g| g.id == goal.id));
}

#[tokio::test]
async fn task_sessions_page_newest_first_with_scoped_cursor_and_404s() {
    let s = setup().await;
    for _ in 0..3 {
        testing::seed_submission(
            s.store.pool(),
            s.task_a,
            s.owner.id,
            "accepted",
            &format_ts(&s.clock.now()),
        )
        .await;
        s.clock.advance(TimeDelta::hours(1));
    }

    let first = s
        .store
        .list_task_sessions(&s.project, &s.task_a, &params(Some(2), None))
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(
        first.items[0].ended_at > first.items[1].ended_at,
        "newest first"
    );
    let cursor = first.next_cursor.clone().unwrap();
    let second = s
        .store
        .list_task_sessions(
            &s.project,
            &s.task_a,
            &params(Some(2), Some(cursor.clone())),
        )
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
    assert!(second.items[0].ended_at < first.items[1].ended_at);

    // The cursor is bound to its task filter.
    assert!(matches!(
        s.store
            .list_task_sessions(&s.project, &s.task_b, &params(Some(2), Some(cursor)))
            .await,
        Err(DomainError::InvalidCursor(_))
    ));

    // Route membership is 404 before anything else.
    let unknown_project = ProjectId::generate(s.clock.now());
    assert!(matches!(
        s.store
            .list_task_sessions(&unknown_project, &s.task_a, &params(None, None))
            .await,
        Err(DomainError::NotFound)
    ));
    let unknown_task = TaskId::generate(s.clock.now());
    assert!(matches!(
        s.store
            .list_task_sessions(&s.project, &unknown_task, &params(None, None))
            .await,
        Err(DomainError::NotFound)
    ));
}

#[tokio::test]
async fn task_view_carries_eligibility_and_selected_plan_ids() {
    let s = setup().await;

    let view = s
        .store
        .task_view(&s.project, &s.task_a, &s.owner)
        .await
        .unwrap();
    assert_eq!(view.task.id, s.task_a);
    assert!(view.selected_plan_revision_ids.is_empty());
    assert!(view.accepted_plan_submission_ids.is_empty());
    assert!(
        view.eligibility.can_plan || view.eligibility.can_execute,
        "a fresh owner task is actionable: {:?}",
        view.eligibility
    );

    // Views surface the stored plan pointers as the contract's 0/1-element arrays.
    let document = testing::create_task_document(
        &s.store,
        &s.owner,
        s.project,
        s.task_a,
        shepherd_core::model::DocumentKind::Plan,
    )
    .await;
    let selected = document.latest_revision_id.as_uuid();
    let accepted: Uuid = testing::seed_submission(
        s.store.pool(),
        s.task_a,
        s.owner.id,
        "accepted",
        &format_ts(&s.clock.now()),
    )
    .await
    .parse()
    .unwrap();
    sqlx::query(
        "UPDATE tasks SET selected_plan_revision_id = ?1, accepted_plan_submission_id = ?2 \
         WHERE id = ?3",
    )
    .bind(selected.to_string())
    .bind(accepted.to_string())
    .bind(s.task_a.to_string())
    .execute(s.store.pool())
    .await
    .unwrap();
    let view = s
        .store
        .task_view(&s.project, &s.task_a, &s.owner)
        .await
        .unwrap();
    assert_eq!(view.selected_plan_revision_ids, vec![selected]);
    assert_eq!(view.accepted_plan_submission_ids, vec![accepted]);

    // Blocking flips eligibility off with a reason.
    s.store
        .block_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            s.project,
            s.task_a,
            "blocked for the view test".into(),
        )
        .await
        .unwrap();
    let view = s
        .store
        .task_view(&s.project, &s.task_a, &s.owner)
        .await
        .unwrap();
    assert!(!view.eligibility.can_plan && !view.eligibility.can_execute);
    assert!(!view.eligibility.reasons.is_empty());

    // Batches preserve input order; unknown ids are NotFound.
    let views = s
        .store
        .task_views(&s.project, &[s.task_b, s.task_a], &s.owner)
        .await
        .unwrap();
    assert_eq!(views[0].task.id, s.task_b);
    assert_eq!(views[1].task.id, s.task_a);
    assert!(matches!(
        s.store
            .task_views(
                &s.project,
                &[s.task_a, TaskId::generate(s.clock.now())],
                &s.owner
            )
            .await,
        Err(DomainError::NotFound)
    ));
}

#[tokio::test]
async fn epic_view_matches_graph_node_eligibility() {
    let s = setup().await;
    let view = s
        .store
        .epic_view(&s.project, &s.epic, &s.owner)
        .await
        .unwrap();
    assert_eq!(view.epic.id, s.epic);

    let graph = s
        .store
        .get_goal_graph(&s.owner, &s.project, &s.goal)
        .await
        .unwrap();
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == s.epic.as_uuid())
        .unwrap();
    assert_eq!(view.eligibility, node.eligibility);

    assert!(matches!(
        s.store
            .epic_view(&s.project, &EpicId::generate(s.clock.now()), &s.owner)
            .await,
        Err(DomainError::NotFound)
    ));
}
