use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Clock, CommandId, Counts, Goal, GoalCreate, GoalId, Project,
    ProjectCreate, ProjectId, ProjectPatch, ProjectSettings, ReviewPolicy, TaskTypeCreate,
    TaskTypePatch, TestClock, TextPatch,
};
use shepherd_core::queries::ListParams;
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, open, testing};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use uuid::Uuid;

async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(testing::store_options(dir.path(), "shepherd.db", clock))
        .await
        .unwrap()
}

// Persisted-state assertions go through an independent read-only connection.
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

async fn table_count(conn: &mut SqliteConnection, table: &str) -> i64 {
    // AssertSqlSafe: table names come from string literals in this file.
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn owner_creates_project_and_two_goals() {
    let s = setup().await;
    let created = s
        .store
        .create_project(
            ctx(&s.owner, s.clock.now(), None),
            ProjectCreate {
                name: "  Space Game  ".to_string(),
                description: Some("Build the space game".to_string()),
                settings: None,
            },
        )
        .await
        .unwrap();
    let project = created.value;
    assert_eq!(project.revision.value(), 1);
    assert_eq!(project.name, "Space Game");
    assert_eq!(project.description, "Build the space game");
    assert_eq!(project.settings, ProjectSettings::default());
    assert!(!project.archived);
    assert!(project.archive.is_none());
    assert_eq!(project.epic_counts, Counts::ZERO);
    assert_eq!(created.events.len(), 7);

    let first = create_goal(&s, project.id, "Ship engine").await;
    let second = create_goal(&s, project.id, "Ship hull").await;
    for goal in [&first, &second] {
        assert_eq!(goal.revision.value(), 1);
        assert_eq!(goal.project_id, project.id);
        assert!(!goal.completed);
        assert_eq!(goal.epic_counts, Counts::ZERO);
    }

    let mut conn = independent_connection(&s.db_path()).await;
    let settings_json: String = sqlx::query_scalar("SELECT settings FROM projects WHERE id = ?1")
        .bind(project.id.to_string())
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        settings_json,
        "{\"proposal_gate\":true,\"planning_required\":false,\
         \"plan_review\":\"human\",\"work_review\":\"human\"}"
    );

    let types: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT key, builtin, revision FROM task_types WHERE project_id = ?1 ORDER BY key",
    )
    .bind(project.id.to_string())
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(
        types
            .iter()
            .map(|(key, ..)| key.as_str())
            .collect::<Vec<_>>(),
        [
            "code",
            "design",
            "documentation",
            "other",
            "research",
            "test"
        ]
    );
    assert!(
        types
            .iter()
            .all(|(_, builtin, revision)| *builtin == 1 && *revision == 1)
    );

    // Create-project events share one command_id: project first, then types by UUID.
    let events: Vec<(String, String, String)> =
        sqlx::query_as("SELECT type, command_id, resource_id FROM events ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_eq!(events.len(), 9);
    let create_events = &events[..7];
    assert!(
        create_events
            .iter()
            .all(|(_, command_id, _)| *command_id == create_events[0].1)
    );
    assert_eq!(create_events[0].0, "project.changed");
    assert_eq!(create_events[0].2, project.id.to_string());
    assert!(
        create_events[1..]
            .iter()
            .all(|(event_type, ..)| event_type == "task_type.changed")
    );
    let type_ids: Vec<&String> = create_events[1..].iter().map(|(.., id)| id).collect();
    let mut sorted_type_ids = type_ids.clone();
    sorted_type_ids.sort();
    assert_eq!(type_ids, sorted_type_ids);
    assert_eq!(events[7].0, "goal.changed");
    assert_eq!(events[8].0, "goal.changed");
    assert_ne!(events[7].1, events[8].1);
    let occurred_at: String = sqlx::query_scalar("SELECT DISTINCT occurred_at FROM events")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(occurred_at, "2026-09-14T00:00:00.000Z");
    conn.close().await.unwrap();
}

#[tokio::test]
async fn agent_cannot_create_project_or_goal() {
    let s = setup().await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "coder");
    register(&s.store, &agent).await;
    let project = create_project(&s, "Owned").await;

    let mut conn = independent_connection(&s.db_path()).await;
    let projects_before = table_count(&mut conn, "projects").await;
    let goals_before = table_count(&mut conn, "goals").await;
    let types_before = table_count(&mut conn, "task_types").await;
    let events_before = table_count(&mut conn, "events").await;

    let err = s
        .store
        .create_project(
            ctx(&agent, s.clock.now(), None),
            ProjectCreate {
                name: "Rogue".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = s
        .store
        .create_goal(
            ctx(&agent, s.clock.now(), None),
            project.id,
            GoalCreate {
                title: "Rogue goal".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = s
        .store
        .create_task_type(
            ctx(&agent, s.clock.now(), None),
            project.id,
            TaskTypeCreate {
                key: "ops".to_string(),
                label: "Ops".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    assert_eq!(table_count(&mut conn, "projects").await, projects_before);
    assert_eq!(table_count(&mut conn, "goals").await, goals_before);
    assert_eq!(table_count(&mut conn, "task_types").await, types_before);
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn empty_goal_reports_zero_counts_and_not_completed() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "Empty").await;

    let fetched = s.store.get_goal(&project.id, &goal.id).await.unwrap();
    assert_eq!(fetched.epic_counts, Counts::ZERO);
    assert!(!fetched.completed);

    let listed = s
        .store
        .list_goals(&project.id, &ListParams::default())
        .await
        .unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].epic_counts, Counts::ZERO);
    assert!(!listed.items[0].completed);
    assert!(listed.next_cursor.is_none());

    let fetched_project = s.store.get_project(&project.id).await.unwrap();
    assert_eq!(fetched_project.epic_counts, Counts::ZERO);
}

#[tokio::test]
async fn duplicate_task_type_key_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let custom = s
        .store
        .create_task_type(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            TaskTypeCreate {
                key: "ops".to_string(),
                label: "Ops".to_string(),
            },
        )
        .await
        .unwrap()
        .value;
    assert!(!custom.builtin);
    assert_eq!(custom.revision.value(), 1);

    let mut conn = independent_connection(&s.db_path()).await;
    let types_before = table_count(&mut conn, "task_types").await;
    let events_before = table_count(&mut conn, "events").await;
    let revisions_before: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, revision FROM task_types ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();

    // Colliding with a seeded builtin and with an existing custom key both fail.
    for key in ["code", "ops"] {
        let err = s
            .store
            .create_task_type(
                ctx(&s.owner, s.clock.now(), None),
                project.id,
                TaskTypeCreate {
                    key: key.to_string(),
                    label: "Again".to_string(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::DuplicateTaskTypeKey { key: duplicate } if duplicate == key
        ));
    }

    assert_eq!(table_count(&mut conn, "task_types").await, types_before);
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    let revisions_after: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, revision FROM task_types ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_eq!(revisions_after, revisions_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn cursor_reused_with_different_filter_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    for title in ["a", "b", "c"] {
        create_goal(&s, project.id, title).await;
        s.clock.advance(TimeDelta::milliseconds(1));
    }
    let params = |cursor: Option<String>, include_archived: bool| ListParams {
        limit: Some(2),
        cursor,
        include_archived,
    };
    let page = s
        .store
        .list_goals(&project.id, &params(None, false))
        .await
        .unwrap();
    let cursor = page.next_cursor.unwrap();

    let err = s
        .store
        .list_goals(&project.id, &params(Some(cursor.clone()), true))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    let err = s
        .store
        .list_projects(&params(Some(cursor.clone()), false))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    let other = create_project(&s, "Q").await;
    let err = s
        .store
        .list_goals(&other.id, &params(Some(cursor.clone()), false))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    let err = s
        .store
        .list_goals(
            &project.id,
            &params(Some("!!!not-base64url!!!".to_string()), false),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    // The unchanged filter still accepts the cursor.
    let rest = s
        .store
        .list_goals(&project.id, &params(Some(cursor), false))
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert!(rest.next_cursor.is_none());
}

#[tokio::test]
async fn cross_project_goal_get_returns_not_found() {
    let s = setup().await;
    let project_a = create_project(&s, "A").await;
    let project_b = create_project(&s, "B").await;
    let goal = create_goal(&s, project_a.id, "In A").await;

    let err = s.store.get_goal(&project_b.id, &goal.id).await.unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    // Membership precedes the revision check: a foreign goal is 404, never 412.
    let err = s
        .store
        .update_goal(
            ctx(&s.owner, s.clock.now(), Some(99)),
            project_b.id,
            goal.id,
            TextPatch {
                title: Some("Stolen".to_string()),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let unchanged = s.store.get_goal(&project_a.id, &goal.id).await.unwrap();
    assert_eq!(unchanged.title, "In A");
    assert_eq!(unchanged.revision.value(), 1);
}

#[tokio::test]
async fn stale_revision_leaves_resource_unchanged() {
    let s = setup().await;
    let project = create_project(&s, "P").await;

    let mut conn = independent_connection(&s.db_path()).await;
    let events_before = table_count(&mut conn, "events").await;

    let patch = || ProjectPatch {
        name: Some("Renamed".to_string()),
        ..Default::default()
    };
    let err = s
        .store
        .update_project(ctx(&s.owner, s.clock.now(), Some(2)), project.id, patch())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::RevisionConflict {
            expected: 2,
            actual: 1
        }
    ));
    let err = s
        .store
        .update_project(ctx(&s.owner, s.clock.now(), None), project.id, patch())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PreconditionRequired));

    let (name, revision, updated_at): (String, i64, String) =
        sqlx::query_as("SELECT name, revision, updated_at FROM projects WHERE id = ?1")
            .bind(project.id.to_string())
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(name, "P");
    assert_eq!(revision, 1);
    assert_eq!(updated_at, "2026-09-14T00:00:00.000Z");
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn update_bumps_revision_exactly_once() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let types = s
        .store
        .list_task_types(&project.id, &ListParams::default())
        .await
        .unwrap()
        .items;
    let code_type = types.iter().find(|t| t.key == "code").unwrap().clone();

    s.clock.advance(TimeDelta::seconds(60));
    let new_settings = ProjectSettings {
        proposal_gate: false,
        planning_required: true,
        plan_review: ReviewPolicy::Agent,
        work_review: ReviewPolicy::None,
    };
    let updated = s
        .store
        .update_project(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            ProjectPatch {
                name: Some("P2".to_string()),
                description: Some("described".to_string()),
                settings: Some(new_settings),
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated.revision.value(), 2);
    assert_eq!(updated.updated_at, s.clock.now());
    assert_eq!(updated.created_at, project.created_at);
    assert_eq!(updated.settings, new_settings);

    let updated_goal = s
        .store
        .update_goal(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            goal.id,
            TextPatch {
                title: Some("G2".to_string()),
                description: Some("described".to_string()),
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(updated_goal.revision.value(), 2);
    assert_eq!(updated_goal.title, "G2");

    let renamed = s
        .store
        .update_task_type(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            code_type.id,
            TaskTypePatch {
                label: Some("Code work".to_string()),
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(renamed.revision.value(), 2);
    assert_eq!(renamed.label, "Code work");
    assert_eq!(renamed.key, "code");
    assert!(renamed.builtin);

    let mut conn = independent_connection(&s.db_path()).await;
    let stored_settings: String = sqlx::query_scalar("SELECT settings FROM projects WHERE id = ?1")
        .bind(project.id.to_string())
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        stored_settings,
        "{\"proposal_gate\":false,\"planning_required\":true,\
         \"plan_review\":\"agent\",\"work_review\":\"none\"}"
    );
    let project_updates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE type = 'project.changed' AND resource_revision = 2",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(project_updates, 1);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn goal_pages_round_trip_with_cursor() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    // g1/g2 and g4/g5 share a millisecond, so paging must tiebreak on id.
    let g1 = create_goal(&s, project.id, "g1").await;
    let g2 = create_goal(&s, project.id, "g2").await;
    s.clock.advance(TimeDelta::milliseconds(5));
    let g3 = create_goal(&s, project.id, "g3").await;
    s.clock.advance(TimeDelta::milliseconds(5));
    let g4 = create_goal(&s, project.id, "g4").await;
    let g5 = create_goal(&s, project.id, "g5").await;

    let params = |cursor: Option<String>| ListParams {
        limit: Some(2),
        cursor,
        include_archived: false,
    };
    let page1 = s
        .store
        .list_goals(&project.id, &params(None))
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 2);
    let page2 = s
        .store
        .list_goals(&project.id, &params(page1.next_cursor.clone()))
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 2);
    let page3 = s
        .store
        .list_goals(&project.id, &params(page2.next_cursor.clone()))
        .await
        .unwrap();
    assert_eq!(page3.items.len(), 1);
    assert!(page3.next_cursor.is_none());

    let collected: Vec<GoalId> = [page1.items, page2.items, page3.items]
        .concat()
        .iter()
        .map(|goal| goal.id)
        .collect();
    assert_eq!(collected, vec![g1.id, g2.id, g3.id, g4.id, g5.id]);
}

#[tokio::test]
async fn invalid_input_is_rejected_without_writes() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let agent = actor(s.clock.now(), ActorKind::Agent, "coder");
    register(&s.store, &agent).await;

    let mut conn = independent_connection(&s.db_path()).await;
    let projects_before = table_count(&mut conn, "projects").await;
    let goals_before = table_count(&mut conn, "goals").await;
    let types_before = table_count(&mut conn, "task_types").await;
    let events_before = table_count(&mut conn, "events").await;

    let long_name = "x".repeat(201);
    for name in ["   ", long_name.as_str()] {
        let err = s
            .store
            .create_project(
                ctx(&s.owner, s.clock.now(), None),
                ProjectCreate {
                    name: name.to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "name", .. }));
    }
    let err = s
        .store
        .create_project(
            ctx(&s.owner, s.clock.now(), None),
            ProjectCreate {
                name: "ok".to_string(),
                description: Some("d".repeat(10_001)),
                settings: None,
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
    let err = s
        .store
        .create_goal(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            GoalCreate {
                title: "  ".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "title", .. }
    ));
    for key in ["Bad", "1abc", "has-dash"] {
        let err = s
            .store
            .create_task_type(
                ctx(&s.owner, s.clock.now(), None),
                project.id,
                TaskTypeCreate {
                    key: key.to_string(),
                    label: "Label".to_string(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "key", .. }));
    }
    let err = s
        .store
        .create_task_type(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            TaskTypeCreate {
                key: "ok".to_string(),
                label: "   ".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "label", .. }
    ));
    for limit in [0, 201] {
        let err = s
            .store
            .list_projects(&ListParams {
                limit: Some(limit),
                cursor: None,
                include_archived: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "limit", .. }
        ));
    }

    // Capability precedes content validity: invalid agent input is still Forbidden.
    let err = s
        .store
        .create_project(
            ctx(&agent, s.clock.now(), None),
            ProjectCreate {
                name: "   ".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    assert_eq!(table_count(&mut conn, "projects").await, projects_before);
    assert_eq!(table_count(&mut conn, "goals").await, goals_before);
    assert_eq!(table_count(&mut conn, "task_types").await, types_before);
    assert_eq!(table_count(&mut conn, "events").await, events_before);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn two_pools_serialize_goal_creation() {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
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

    let mut handles = Vec::new();
    for round in 0..4 {
        for (pool_name, store) in [("a", store_a.clone()), ("b", store_b.clone())] {
            let owner = owner.clone();
            let clock = clock.clone();
            let project_id = project.id;
            handles.push(tokio::spawn(async move {
                store
                    .create_goal(
                        ctx(&owner, clock.now(), None),
                        project_id,
                        GoalCreate {
                            title: format!("goal-{pool_name}-{round}"),
                            description: None,
                        },
                    )
                    .await
            }));
        }
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    let goals = store_a
        .list_goals(&project.id, &ListParams::default())
        .await
        .unwrap()
        .items;
    assert_eq!(goals.len(), 8);
    let mut conn = independent_connection(&dir.path().join("shepherd.db")).await;
    let distinct_commands: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT command_id) FROM events WHERE type = 'goal.changed'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(distinct_commands, 8);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn include_archived_filter_controls_visibility() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let live = create_goal(&s, project.id, "live").await;

    // Direct-SQL fixture: no public command can archive until step 006.
    let archived_goal = GoalId::generate(s.clock.now());
    let mut conn = s.store.pool().acquire().await.unwrap();
    sqlx::query(
        "INSERT INTO goals (id, revision, created_at, updated_at, project_id, title, \
         description, archived, archive_actor_id, archive_reason, archive_created_at) \
         VALUES (?1, 1, ?2, ?2, ?3, 'archived goal', '', 1, ?4, 'no longer needed', ?2)",
    )
    .bind(archived_goal.to_string())
    .bind(format_ts(&s.clock.now()))
    .bind(project.id.to_string())
    .bind(s.owner.id.to_string())
    .execute(&mut *conn)
    .await
    .unwrap();
    drop(conn);

    let default_page = s
        .store
        .list_goals(&project.id, &ListParams::default())
        .await
        .unwrap();
    assert_eq!(
        default_page.items.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![live.id]
    );

    let all_page = s
        .store
        .list_goals(
            &project.id,
            &ListParams {
                include_archived: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(all_page.items.len(), 2);
    let archived_item = all_page
        .items
        .iter()
        .find(|g| g.id == archived_goal)
        .unwrap();
    assert!(archived_item.archived);
    let record = archived_item.archive.as_ref().unwrap();
    assert_eq!(record.reason, "no longer needed");
    assert_eq!(record.actor_id, s.owner.id);
    assert!(
        all_page
            .items
            .iter()
            .all(|g| g.epic_counts == Counts::ZERO && !g.completed)
    );
}
