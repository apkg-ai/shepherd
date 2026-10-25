use chrono::TimeDelta;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    ClaimPhase, Clock, DependencyCreate, DocumentKind, DocumentOwner, ReviewDecision, ReviewPolicy,
    SessionOutcome, TaskId,
};
use uuid::Uuid;

use crate::fixtures::*;

// --- Acceptance sentence 1: a different agent receives the exact accepted
// --- plan and prior feedback, not just the latest mutable note.

#[tokio::test]
async fn context_returns_pinned_accepted_plan_not_latest_revision() {
    let s = setup().await;
    let fixture = accepted_plan_with_newer_revision(&s).await;

    // A different agent's view is the same snapshot query.
    let bundle = s
        .store
        .get_task_context(&fixture.project, &fixture.task.id)
        .await
        .unwrap();

    assert_eq!(bundle.selected_plan.len(), 1);
    assert_eq!(bundle.selected_plan[0].id.as_uuid(), fixture.first_revision);
    assert_eq!(bundle.selected_plan[0].body, "plan v1");
    assert_eq!(
        bundle.selected_plan_revision_ids,
        vec![fixture.first_revision]
    );
    assert_eq!(bundle.accepted_plan_submission_ids.len(), 1);

    // The document's mutable latest moved on; the pinned revision did not.
    let latest: String =
        sqlx::query_scalar("SELECT latest_revision_id FROM documents WHERE id = ?1")
            .bind(fixture.plan_document.id.to_string())
            .fetch_one(s.store.pool())
            .await
            .unwrap();
    assert_ne!(latest, fixture.first_revision.to_string());
    assert!(!bundle.truncated);
}

#[tokio::test]
async fn context_includes_prior_reviews_and_rejection_feedback() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::Human, ReviewPolicy::None).await;
    let producer = agent(&s, "producer").await;

    // First plan attempt: rejected with durable feedback.
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Plan).await;
    let document = create_document(
        &s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Plan,
        "plan v1",
    )
    .await;
    let first = document.latest_revision_id.as_uuid();
    select_plan(&s, project, task.id, first).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "planned",
        vec![first],
    )
    .await
    .unwrap();
    let rejected = pending_submission_id(&s.store, task.id).await;
    review(
        &s,
        &s.owner,
        project,
        rejected,
        ReviewDecision::Reject,
        "needs risk analysis",
        None,
    )
    .await;

    // Second attempt: revised plan accepted.
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Plan).await;
    let second = add_revision(&s, &producer, project, &document, 1, "plan v2")
        .await
        .id
        .as_uuid();
    select_plan(&s, project, task.id, second).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "replanned",
        vec![second],
    )
    .await
    .unwrap();
    let accepted = pending_submission_id(&s.store, task.id).await;
    review(
        &s,
        &s.owner,
        project,
        accepted,
        ReviewDecision::Approve,
        "solid",
        None,
    )
    .await;

    let bundle = s.store.get_task_context(&project, &task.id).await.unwrap();

    // Newest first: the approval precedes the rejection; feedback is intact.
    assert_eq!(bundle.reviews.len(), 2);
    assert_eq!(bundle.reviews[0].decision, ReviewDecision::Approve);
    assert_eq!(bundle.reviews[1].decision, ReviewDecision::Reject);
    assert_eq!(bundle.reviews[1].reason, "needs risk analysis");
    assert_eq!(bundle.submissions.len(), 2);
    assert_eq!(bundle.submissions[0].id, accepted);
    assert_eq!(bundle.submissions[1].id, rejected);
    assert_eq!(bundle.sessions.len(), 2);
    assert_eq!(bundle.selected_plan[0].id.as_uuid(), second);
}

#[tokio::test]
async fn context_scopes_sections_to_task_epic_and_project() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic_one = create_epic(&s, project.id, goal, "E1").await;
    let epic_two = create_epic(&s, project.id, goal, "E2").await;
    let epic_three = create_epic(&s, project.id, goal, "E3").await;
    let target = create_task_full(
        &s,
        project.id,
        epic_one.id,
        "T1",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let sibling = create_task_full(
        &s,
        project.id,
        epic_one.id,
        "T2",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let bystander = create_task_full(
        &s,
        project.id,
        epic_one.id,
        "T3",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;

    // Documents: only project- and this-epic-owned revisions belong in context.
    let project_doc = create_document(
        &s,
        project.id,
        DocumentOwner::Project(project.id),
        DocumentKind::Brief,
        "project brief",
    )
    .await;
    let epic_doc = create_document(
        &s,
        project.id,
        DocumentOwner::Epic(epic_one.id),
        DocumentKind::Note,
        "epic note",
    )
    .await;
    create_document(
        &s,
        project.id,
        DocumentOwner::Epic(epic_two.id),
        DocumentKind::Note,
        "other epic note",
    )
    .await;
    create_document(
        &s,
        project.id,
        DocumentOwner::Task(target.id),
        DocumentKind::Note,
        "task note",
    )
    .await;
    // Revising the older project doc makes its content the newest: documents
    // order by latest-revision recency, not document creation time.
    add_revision(
        &s,
        &s.owner,
        project.id,
        &project_doc,
        1,
        "project brief v2",
    )
    .await;

    // Sibling activity on T2 must stay out of T1's sessions/submissions.
    let worker = agent(&s, "worker").await;
    failed_attempts(&s, &worker, project.id, sibling.id, 1, "sibling attempt").await;

    // Dependencies: links touching E1 or T1 belong; others do not.
    let deps = [
        DependencyCreate::Epic {
            dependent_id: epic_one.id,
            prerequisite_id: epic_two.id,
        },
        DependencyCreate::Epic {
            dependent_id: epic_two.id,
            prerequisite_id: epic_three.id,
        },
        DependencyCreate::Task {
            dependent_id: target.id,
            prerequisite_id: sibling.id,
        },
        DependencyCreate::Task {
            dependent_id: sibling.id,
            prerequisite_id: bystander.id,
        },
    ];
    for dep in deps {
        s.clock.advance(TimeDelta::milliseconds(2));
        s.store
            .create_dependency(ctx(&s.owner, s.clock.now(), None), project.id, dep)
            .await
            .unwrap();
    }

    let bundle = s
        .store
        .get_task_context(&project.id, &target.id)
        .await
        .unwrap();

    let document_ids: Vec<Uuid> = bundle
        .documents
        .iter()
        .map(|revision| revision.document_id.as_uuid())
        .collect();
    assert_eq!(
        document_ids,
        vec![project_doc.id.as_uuid(), epic_doc.id.as_uuid()],
        "newest revision first, project/epic owners only"
    );
    assert_eq!(bundle.documents[0].body, "project brief v2");
    assert!(bundle.sessions.is_empty(), "sibling sessions excluded");
    assert!(bundle.submissions.is_empty());
    let dependency_ends: Vec<(Uuid, Uuid)> = bundle
        .dependencies
        .iter()
        .map(|dep| (dep.dependent_id, dep.prerequisite_id))
        .collect();
    assert_eq!(
        dependency_ends,
        vec![
            (epic_one.id.as_uuid(), epic_two.id.as_uuid()),
            (target.id.as_uuid(), sibling.id.as_uuid()),
        ]
    );
    assert!(!bundle.truncated);
}

#[tokio::test]
async fn context_sibling_claims_are_effective_active_claims_on_other_tasks() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let none = ReviewPolicy::None;
    let target = create_task_full(&s, project.id, epic.id, "T1", false, none, none).await;
    let sibling = create_task_full(&s, project.id, epic.id, "T2", false, none, none).await;
    let stale = create_task_full(&s, project.id, epic.id, "T3", false, none, none).await;

    let worker_a = agent(&s, "worker-a").await;
    let worker_b = agent(&s, "worker-b").await;
    let worker_c = agent(&s, "worker-c").await;

    // T3's claim will expire (default TTL 300s, no sweep); T2's stays live.
    claim(&s, &worker_c, project.id, stale.id, ClaimPhase::Execute).await;
    s.clock.advance(TimeDelta::seconds(250));
    let own_claim = claim(&s, &worker_a, project.id, target.id, ClaimPhase::Execute).await;
    let live = claim(&s, &worker_b, project.id, sibling.id, ClaimPhase::Execute).await;
    s.clock.advance(TimeDelta::seconds(100));

    let bundle = s
        .store
        .get_task_context(&project.id, &target.id)
        .await
        .unwrap();

    assert_eq!(bundle.sibling_claims.len(), 1);
    assert_eq!(bundle.sibling_claims[0].id, live.claim.id);
    assert_eq!(bundle.sibling_claims[0].task_id, sibling.id);
    assert_ne!(bundle.sibling_claims[0].id, own_claim.claim.id);
}

#[tokio::test]
async fn context_unknown_or_cross_project_task_is_not_found() {
    let s = setup().await;
    let (project_a, _, task_a) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let project_b = create_project(&s, "Other").await;

    let unknown = TaskId::from_uuid(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)));
    assert!(matches!(
        s.store.get_task_context(&project_a, &unknown).await,
        Err(DomainError::NotFound)
    ));
    assert!(matches!(
        s.store.get_task_context(&project_b.id, &task_a.id).await,
        Err(DomainError::NotFound)
    ));
}

#[tokio::test]
async fn context_remains_readable_on_archived_scope() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project,
            task.id,
            "descoped".into(),
        )
        .await
        .unwrap();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .archive_task(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(task_revision(&s.store, task.id).await),
            ),
            project,
            task.id,
            "superseded".into(),
        )
        .await
        .unwrap();

    let bundle = s.store.get_task_context(&project, &task.id).await.unwrap();
    assert!(bundle.task.archived);
    assert!(!bundle.truncated);
}

// --- Acceptance sentence 2: context truncation is explicit and has
// --- continuation links.

#[tokio::test]
async fn context_truncates_documents_with_exact_continuation_paths() {
    let s = setup().await;
    let (project, epic, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let mut documents = Vec::new();
    for index in 0..5 {
        let document = create_document(
            &s,
            project,
            DocumentOwner::Epic(epic.id),
            DocumentKind::Note,
            &format!("{index}{}", "x".repeat(60_000)),
        )
        .await;
        documents.push(document);
    }

    let bundle = s.store.get_task_context(&project, &task.id).await.unwrap();

    // Four 60K-plus revisions fit the 256 KiB budget; the oldest is trimmed.
    assert!(bundle.truncated);
    assert_eq!(bundle.documents.len(), 4);
    let kept: Vec<Uuid> = bundle
        .documents
        .iter()
        .map(|revision| revision.document_id.as_uuid())
        .collect();
    let expected: Vec<Uuid> = documents
        .iter()
        .rev()
        .take(4)
        .map(|document| document.id.as_uuid())
        .collect();
    assert_eq!(kept, expected, "newest four retained");
    let paths = context_paths(project, epic.id, task.id);
    assert_eq!(
        bundle.continuation_paths,
        vec![paths[2].clone(), paths[3].clone()],
        "exact collection continuation paths for both document owners"
    );
    assert!(bundle.sessions.is_empty() && bundle.reviews.is_empty());
}

#[tokio::test]
async fn context_truncation_drops_lower_priority_sections_first() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E1").await;
    let epic_two = create_epic(&s, project.id, goal, "E2").await;
    let none = ReviewPolicy::None;
    let task = create_task_full(&s, project.id, epic.id, "T1", false, none, none).await;
    let sibling = create_task_full(&s, project.id, epic.id, "T2", false, none, none).await;

    // Twenty 10K-character sessions consume most of the budget.
    let worker = agent(&s, "worker").await;
    failed_attempts(&s, &worker, project.id, task.id, 20, &"s".repeat(10_000)).await;

    // Lower-priority content that no longer fits: a 60K document, a sibling
    // claim and a dependency.
    create_document(
        &s,
        project.id,
        DocumentOwner::Epic(epic.id),
        DocumentKind::Note,
        &"x".repeat(60_000),
    )
    .await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Epic {
                dependent_id: epic_two.id,
                prerequisite_id: epic.id,
            },
        )
        .await
        .unwrap();
    let sibling_worker = agent(&s, "sibling-worker").await;
    claim(
        &s,
        &sibling_worker,
        project.id,
        sibling.id,
        ClaimPhase::Execute,
    )
    .await;

    let bundle = s
        .store
        .get_task_context(&project.id, &task.id)
        .await
        .unwrap();

    assert_eq!(bundle.sessions.len(), 20, "highest priority retained");
    assert!(bundle.documents.is_empty(), "documents no longer fit");
    assert!(bundle.sibling_claims.is_empty());
    assert!(bundle.dependencies.is_empty());
    assert!(bundle.truncated);
    let paths = context_paths(project.id, epic.id, task.id);
    assert_eq!(
        bundle.continuation_paths,
        vec![
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
            paths[5].clone(),
        ]
    );
}

#[tokio::test]
async fn context_count_caps_set_truncated_with_continuations() {
    let s = setup().await;
    let (project, epic, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let worker = agent(&s, "worker").await;
    failed_attempts(&s, &worker, project, task.id, 25, "attempt").await;

    let bundle = s.store.get_task_context(&project, &task.id).await.unwrap();

    assert_eq!(bundle.sessions.len(), 20, "contract cap");
    // Newest first: strictly descending end times.
    for pair in bundle.sessions.windows(2) {
        assert!(pair[0].ended_at > pair[1].ended_at);
    }
    assert!(bundle.truncated);
    let paths = context_paths(project, epic.id, task.id);
    assert_eq!(bundle.continuation_paths, vec![paths[0].clone()]);
}

#[tokio::test]
async fn context_mandatory_plus_plan_over_budget_is_context_too_large() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::None, ReviewPolicy::None).await;

    // 65,536 four-byte characters: the maximum-length plan body alone reaches
    // 256 KiB of UTF-8 JSON before the mandatory entities are added.
    let document = create_document(
        &s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Plan,
        &"\u{1F4A5}".repeat(65_536),
    )
    .await;
    let revision = document.latest_revision_id.as_uuid();
    select_plan(&s, project, task.id, revision).await;

    let err = s
        .store
        .get_task_context(&project, &task.id)
        .await
        .unwrap_err();
    match &err {
        DomainError::ContextTooLarge { continuation } => {
            assert_eq!(
                continuation,
                &format!("/api/v1/projects/{project}/document-revisions/{revision}")
            );
        }
        other => panic!("expected ContextTooLarge, got {other:?}"),
    }
    assert_eq!(err.code(), "context_too_large");
}

#[tokio::test]
async fn context_minimal_bundle_is_not_truncated() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;

    let bundle = s.store.get_task_context(&project, &task.id).await.unwrap();

    assert!(bundle.selected_plan.is_empty());
    assert!(bundle.selected_plan_revision_ids.is_empty());
    assert!(bundle.accepted_plan_submission_ids.is_empty());
    assert!(bundle.documents.is_empty());
    assert!(bundle.submissions.is_empty());
    assert!(bundle.reviews.is_empty());
    assert!(bundle.sessions.is_empty());
    assert!(bundle.dependencies.is_empty());
    assert!(bundle.sibling_claims.is_empty());
    assert!(!bundle.truncated);
    assert!(bundle.continuation_paths.is_empty());
}
