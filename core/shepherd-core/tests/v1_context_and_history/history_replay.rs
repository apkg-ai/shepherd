use chrono::TimeDelta;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Clock, DocumentCreate, DocumentKind, DocumentOwner, GoalCreate, ProjectId, ReviewPolicy,
    TextPatch,
};
use shepherd_core::queries::{ReplayOutcome, ResyncReason};
use uuid::Uuid;

use crate::fixtures::*;

// --- Acceptance sentence 4: event IDs survive restart; duplicate delivery
// --- has stable IDs.

#[tokio::test]
async fn event_ids_survive_restart_and_resume_monotonically() {
    let s = setup().await;
    let (project, _, _) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let before = events_after(&s.store, 0).await;
    let max_before = last_event_id(&s.store).await;

    s.store.pool().close().await;
    drop(s.store);
    let store = open_store(&s.dir, s.clock.clone()).await;

    // Byte-identical rows after reopening the same database file.
    let after: Vec<EventRow> = sqlx::query_as(
        "SELECT id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, \
         action, reason FROM events WHERE id > 0 ORDER BY id ASC",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    assert_eq!(after, before);

    // New ids continue strictly after the persisted maximum.
    s.clock.advance(TimeDelta::milliseconds(2));
    store
        .create_goal(
            ctx(&s.owner, s.clock.now(), None),
            project,
            GoalCreate {
                title: "After restart".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    let fresh: Vec<EventRow> = sqlx::query_as(
        "SELECT id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, \
         action, reason FROM events WHERE id > ?1 ORDER BY id ASC",
    )
    .bind(max_before)
    .fetch_all(store.pool())
    .await
    .unwrap();
    assert!(!fresh.is_empty());
    assert!(fresh.iter().all(|row| row.0 > max_before));
}

#[tokio::test]
async fn idempotent_replay_appends_no_duplicate_events() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    s.clock.advance(TimeDelta::milliseconds(2));
    let input = || DocumentCreate {
        owner: DocumentOwner::Task(task.id),
        kind: DocumentKind::Note,
        title: "note".into(),
        body: "body".into(),
        links: Vec::new(),
    };
    let first = s
        .store
        .create_document(
            ctx_with_key(&s.owner, s.clock.now(), key),
            project,
            input(),
            "same-hash",
        )
        .await
        .unwrap();
    assert!(!first.is_replay());
    let rows = events_after(&s.store, 0).await;

    // Duplicate delivery: same key, same request, stable response and history.
    s.clock.advance(TimeDelta::milliseconds(2));
    let second = s
        .store
        .create_document(
            ctx_with_key(&s.owner, s.clock.now(), key),
            project,
            input(),
            "same-hash",
        )
        .await
        .unwrap();
    assert!(second.is_replay());
    assert_eq!(second.into_inner().id, first.into_inner().id);
    assert_eq!(events_after(&s.store, 0).await, rows);
}

#[tokio::test]
async fn history_and_replay_rereads_return_identical_ids() {
    let s = setup().await;
    let (project, _, _) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let ids = |page: &shepherd_core::queries::Page<shepherd_core::model::Event>| {
        page.items.iter().map(|event| event.id).collect::<Vec<_>>()
    };
    let first = s
        .store
        .list_history(&project, None, &history_params(None, None))
        .await
        .unwrap();
    let second = s
        .store
        .list_history(&project, None, &history_params(None, None))
        .await
        .unwrap();
    assert!(!first.items.is_empty());
    assert_eq!(ids(&first), ids(&second));

    let replay_ids = |outcome: &ReplayOutcome| match outcome {
        ReplayOutcome::Events(events) => events.iter().map(|event| event.id).collect::<Vec<_>>(),
        other => panic!("expected events, got {other:?}"),
    };
    let first_replay = s.store.replay_events(&project, 0).await.unwrap();
    let second_replay = s.store.replay_events(&project, 0).await.unwrap();
    assert_eq!(replay_ids(&first_replay), replay_ids(&second_replay));

    // History (newest first) and replay (oldest first) agree on identity.
    let mut ascending = ids(&first);
    ascending.reverse();
    assert_eq!(ascending, replay_ids(&first_replay));
}

// --- History pagination and negative scenarios ---

#[tokio::test]
async fn history_lists_newest_first_and_paginates() {
    let s = setup().await;
    let (project, _) = seeded_history(&s).await;
    let total = event_count(&s.store).await as usize;
    assert!(total > 50);

    let first = s
        .store
        .list_history(&project, None, &history_params(None, None))
        .await
        .unwrap();
    assert_eq!(first.items.len(), 50, "default page size");
    assert!(first.next_cursor.is_some());

    // Walk every page: strictly descending ids, no duplicates, exhaustive.
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = s
            .store
            .list_history(&project, None, &history_params(None, cursor))
            .await
            .unwrap();
        seen.extend(page.items.iter().map(|event| event.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen.len(), total);
    for pair in seen.windows(2) {
        assert!(pair[0] > pair[1], "newest first across pages");
    }

    for out_of_range in [0, 201] {
        let err = s
            .store
            .list_history(&project, None, &history_params(Some(out_of_range), None))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "limit", .. }
        ));
    }
}

#[tokio::test]
async fn history_resource_filter_binds_cursor_fingerprint() {
    let s = setup().await;
    let (project, goal) = seeded_history(&s).await;

    let filtered = s
        .store
        .list_history(
            &project,
            Some(goal.as_uuid()),
            &history_params(Some(10), None),
        )
        .await
        .unwrap();
    assert_eq!(filtered.items.len(), 10);
    assert!(
        filtered
            .items
            .iter()
            .all(|event| event.resource_id == goal.as_uuid())
    );
    let cursor = filtered.next_cursor.unwrap();

    // The same cursor outside its filter or endpoint fingerprint is invalid.
    let err = s
        .store
        .list_history(&project, None, &history_params(None, Some(cursor.clone())))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    let other = create_project(&s, "Other").await;
    let err = s
        .store
        .list_history(
            &other.id,
            Some(goal.as_uuid()),
            &history_params(None, Some(cursor.clone())),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    let err = s
        .store
        .list_history(
            &project,
            Some(goal.as_uuid()),
            &history_params(None, Some("not a cursor".into())),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    // The cursor continues the filtered walk when used correctly.
    let next = s
        .store
        .list_history(
            &project,
            Some(goal.as_uuid()),
            &history_params(Some(10), Some(cursor)),
        )
        .await
        .unwrap();
    assert!(!next.items.is_empty());
    assert!(next.items[0].id < filtered.items[9].id);
}

#[tokio::test]
async fn history_unknown_project_is_not_found_before_cursor_validation() {
    let s = setup().await;
    scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let unknown = ProjectId::from_uuid(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)));
    let err = s
        .store
        .list_history(
            &unknown,
            None,
            &history_params(None, Some("garbage-cursor".into())),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn history_cross_project_resource_filter_leaks_nothing() {
    let s = setup().await;
    let (project_a, _, _) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let project_b = create_project(&s, "Other").await;
    let goal_b = create_goal(&s, project_b.id, "Their goal").await;

    let page = s
        .store
        .list_history(
            &project_a,
            Some(goal_b.as_uuid()),
            &history_params(None, None),
        )
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
}

// --- Bounded replay, independent from HTTP streaming ---

#[tokio::test]
async fn replay_returns_ascending_project_events_after_cursor() {
    let s = setup().await;
    let project_a = create_project(&s, "A").await;
    let project_b = create_project(&s, "B").await;
    let goal_a = create_goal(&s, project_a.id, "GA").await;
    create_goal(&s, project_b.id, "GB").await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .update_goal(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project_a.id,
            goal_a,
            TextPatch {
                title: Some("GA2".into()),
                description: None,
            },
        )
        .await
        .unwrap();

    let outcome = s.store.replay_events(&project_a.id, 0).await.unwrap();
    let events = match outcome {
        ReplayOutcome::Events(events) => events,
        other => panic!("expected events, got {other:?}"),
    };
    let expected: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM events WHERE project_id = ?1 ORDER BY id ASC")
            .bind(project_a.id.to_string())
            .fetch_all(s.store.pool())
            .await
            .unwrap();
    let ids: Vec<i64> = events.iter().map(|event| event.id).collect();
    assert_eq!(ids, expected);
    // Interleaved project B events leave valid numeric gaps.
    let max = *ids.last().unwrap();
    assert!(ids.len() < max as usize, "gaps from the other project");
    assert!(events.iter().all(|event| event.project_id == project_a.id));

    // A mid-stream cursor resumes after that id.
    let resumed = s.store.replay_events(&project_a.id, ids[2]).await.unwrap();
    match resumed {
        ReplayOutcome::Events(events) => {
            assert_eq!(
                events.iter().map(|event| event.id).collect::<Vec<_>>(),
                ids[3..].to_vec()
            );
        }
        other => panic!("expected events, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_cursor_ahead_requires_resync() {
    let s = setup().await;
    let (project, _, _) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let high_water = s.store.event_high_water().await.unwrap();
    assert!(high_water > 0);

    let outcome = s
        .store
        .replay_events(&project, high_water + 5)
        .await
        .unwrap();
    match outcome {
        ReplayOutcome::ResyncRequired {
            current_cursor,
            reason,
        } => {
            assert_eq!(current_cursor, high_water);
            assert_eq!(reason, ResyncReason::CursorAhead);
        }
        other => panic!("expected resync, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_cursor_older_than_seven_days_requires_resync() {
    let s = setup().await;
    let (project, _, _) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let old_max = s.store.event_high_water().await.unwrap();

    // Eight days later the original cursor predates the replay window.
    s.clock.advance(TimeDelta::days(8));
    let outcome = s.store.replay_events(&project, 0).await.unwrap();
    match outcome {
        ReplayOutcome::ResyncRequired {
            current_cursor,
            reason,
        } => {
            assert_eq!(current_cursor, old_max);
            assert_eq!(reason, ResyncReason::CursorExpired);
        }
        other => panic!("expected resync, got {other:?}"),
    }

    // A cursor at the retained maximum still replays fresh events.
    s.clock.advance(TimeDelta::milliseconds(2));
    create_goal(&s, project, "Fresh").await;
    let outcome = s.store.replay_events(&project, old_max).await.unwrap();
    match outcome {
        ReplayOutcome::Events(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].event_type,
                shepherd_core::model::EventType::GoalChanged
            );
        }
        other => panic!("expected events, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_backlog_over_limit_requires_resync() {
    let s = setup().await;
    let project_full = create_project(&s, "Full").await;
    let project_over = create_project(&s, "Over").await;
    let base_full = s.store.event_high_water().await.unwrap();
    seed_events(&s, project_full.id, 1_000).await;
    let cursor_full = base_full;
    seed_events(&s, project_over.id, 1_001).await;

    // Exactly the bound replays in full.
    let outcome = s
        .store
        .replay_events(&project_full.id, cursor_full)
        .await
        .unwrap();
    match outcome {
        ReplayOutcome::Events(events) => assert_eq!(events.len(), 1_000),
        other => panic!("expected events, got {other:?}"),
    }

    // One past the bound demands a resync.
    let outcome = s.store.replay_events(&project_over.id, 0).await.unwrap();
    match outcome {
        ReplayOutcome::ResyncRequired { reason, .. } => {
            assert_eq!(reason, ResyncReason::BacklogLimit);
        }
        other => panic!("expected resync, got {other:?}"),
    }
}

// --- Concurrency over independent pools on one file ---

#[tokio::test]
async fn concurrent_commands_produce_disjoint_coherent_event_groups() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let store_b = open_store(&s.dir, s.clock.clone()).await;
    let marker = last_event_id(&s.store).await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let now = s.clock.now();
    let goal = |title: &str| GoalCreate {
        title: title.to_string(),
        description: None,
    };
    let (first, second) = tokio::join!(
        s.store
            .create_goal(ctx(&s.owner, now, None), project.id, goal("From pool A")),
        store_b.create_goal(ctx(&s.owner, now, None), project.id, goal("From pool B")),
    );
    first.unwrap();
    second.unwrap();

    let rows = events_after(&s.store, marker).await;
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].4, rows[1].4, "distinct command groups");
    let mut ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    ids.dedup();
    assert_eq!(ids.len(), 2, "unique event ids across pools");
    for row in &rows {
        assert_group(std::slice::from_ref(row), &["goal.changed"], true);
    }
}
