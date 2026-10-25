use chrono::TimeDelta;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    ClaimInput, ClaimPhase, Clock, DependencyCreate, DocumentKind, DocumentOwner, ReviewDecision,
    ReviewPolicy, SessionOutcome, TextPatch,
};
use shepherd_core::storage::failpoint;

use crate::fixtures::*;

// --- Acceptance sentence 3: every successful command has a coherent event
// --- group; a failed transaction has none.

#[tokio::test]
async fn events_cover_creation_acceptance_and_update_transitions() {
    let s = setup().await;

    let marker = last_event_id(&s.store).await;
    let project = create_project(&s, "P").await;
    let rows = events_after(&s.store, marker).await;
    assert_group(
        &rows,
        &[
            "project.changed",
            "task_type.changed",
            "task_type.changed",
            "task_type.changed",
            "task_type.changed",
            "task_type.changed",
            "task_type.changed",
        ],
        true,
    );

    let marker = last_event_id(&s.store).await;
    let goal = create_goal(&s, project.id, "G").await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["goal.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["epic.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    let task = create_task_full(
        &s,
        project.id,
        epic.id,
        "T",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .update_goal(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            goal,
            TextPatch {
                title: Some("Goal".into()),
                description: None,
            },
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["goal.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            shepherd_core::model::TaskPatch {
                title: Some("Task".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed"],
        true,
    );
}

#[tokio::test]
async fn events_cover_claim_report_and_review_transitions() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::Agent, ReviewPolicy::Agent).await;
    let producer = agent(&s, "producer").await;
    let reviewer = agent(&s, "reviewer").await;

    // Claim plan.
    let marker = last_event_id(&s.store).await;
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Plan).await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed", "claim.changed"],
        true,
    );

    // Renew.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .renew_claim(
            ctx(&producer, s.clock.now(), None),
            project,
            grant.claim.id,
            &grant.lease_token,
            shepherd_core::model::RenewInput { ttl_seconds: None },
            "renew-hash",
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["claim.changed"],
        true,
    );

    // Plan report failed.
    let marker = last_event_id(&s.store).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Failed,
        "missing inputs",
        Vec::new(),
    )
    .await
    .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["session.recorded", "task.changed", "claim.changed"],
        true,
    );

    // Plan report succeeded.
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Plan).await;
    let document = create_document(
        &s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Plan,
        "plan",
    )
    .await;
    let revision = document.latest_revision_id.as_uuid();
    select_plan(&s, project, task.id, revision).await;
    let marker = last_event_id(&s.store).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "planned",
        vec![revision],
    )
    .await
    .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &[
            "session.recorded",
            "submission.changed",
            "task.changed",
            "claim.changed",
        ],
        true,
    );
    let plan_submission = pending_submission_id(&s.store, task.id).await;

    // Claim review (agent gate): claim.changed only.
    s.clock.advance(TimeDelta::milliseconds(2));
    let marker = last_event_id(&s.store).await;
    let review_grant = s
        .store
        .claim_task(
            ctx(
                &reviewer,
                s.clock.now(),
                Some(task_revision(&s.store, task.id).await),
            ),
            project,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Review,
                submission_id: Some(plan_submission),
                ttl_seconds: None,
            },
            "review-claim-hash",
        )
        .await
        .unwrap()
        .into_inner();
    assert_group(
        &events_after(&s.store, marker).await,
        &["claim.changed"],
        true,
    );

    // Approve plan with the review claim.
    let marker = last_event_id(&s.store).await;
    review(
        &s,
        &reviewer,
        project,
        plan_submission,
        ReviewDecision::Approve,
        "ship it",
        Some((review_grant.claim.id, &review_grant.lease_token)),
    )
    .await;
    assert_group(
        &events_after(&s.store, marker).await,
        &[
            "review.recorded",
            "submission.changed",
            "task.changed",
            "claim.changed",
        ],
        true,
    );

    // Claim execute: epic goes active.
    let marker = last_event_id(&s.store).await;
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Execute).await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed", "epic.changed", "claim.changed"],
        true,
    );

    // Execute report failed.
    let marker = last_event_id(&s.store).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Failed,
        "flaky",
        Vec::new(),
    )
    .await
    .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["session.recorded", "task.changed", "claim.changed"],
        true,
    );

    // Execute report succeeded with a task-owned output: work submission pending.
    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Execute).await;
    let output = create_document(
        &s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Finding,
        "result",
    )
    .await;
    let marker = last_event_id(&s.store).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "built",
        vec![output.latest_revision_id.as_uuid()],
    )
    .await
    .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &[
            "session.recorded",
            "submission.changed",
            "task.changed",
            "claim.changed",
        ],
        true,
    );
    let work_submission = pending_submission_id(&s.store, task.id).await;

    // Withdraw, then resubmit for the approval path.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .withdraw_submission(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(submission_revision(&s.store, work_submission).await),
            ),
            project,
            work_submission,
            "resubmitting".to_string(),
            "withdraw-hash",
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["submission.changed", "task.changed"],
        false,
    );

    let grant = claim(&s, &producer, project, task.id, ClaimPhase::Execute).await;
    report(
        &s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "rebuilt",
        vec![output.latest_revision_id.as_uuid()],
    )
    .await
    .unwrap();
    let work_submission = pending_submission_id(&s.store, task.id).await;

    // Approve work: task completes and the single-task epic cascades to done.
    s.clock.advance(TimeDelta::milliseconds(2));
    let review_grant = s
        .store
        .claim_task(
            ctx(
                &reviewer,
                s.clock.now(),
                Some(task_revision(&s.store, task.id).await),
            ),
            project,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Review,
                submission_id: Some(work_submission),
                ttl_seconds: None,
            },
            "review-claim-hash-2",
        )
        .await
        .unwrap()
        .into_inner();
    let marker = last_event_id(&s.store).await;
    review(
        &s,
        &reviewer,
        project,
        work_submission,
        ReviewDecision::Approve,
        "verified",
        Some((review_grant.claim.id, &review_grant.lease_token)),
    )
    .await;
    assert_group(
        &events_after(&s.store, marker).await,
        &[
            "review.recorded",
            "submission.changed",
            "task.changed",
            "claim.changed",
            "epic.changed",
        ],
        false,
    );
}

#[tokio::test]
async fn events_cover_release_block_cancel_waive_and_completion_transitions() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let none = ReviewPolicy::None;
    let task_one = create_task_full(&s, project.id, epic.id, "T1", false, none, none).await;
    let task_two = create_task_full(&s, project.id, epic.id, "T2", false, none, none).await;
    let worker = agent(&s, "worker").await;

    // Release.
    let grant = claim(&s, &worker, project.id, task_one.id, ClaimPhase::Execute).await;
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .release_claim(
            ctx(&worker, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            "release-hash",
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["claim.changed"],
        false,
    );

    // Block task while claimed: the claim is revoked in the same group.
    claim(&s, &worker, project.id, task_one.id, ClaimPhase::Execute).await;
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .block_task(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(task_revision(&s.store, task_one.id).await),
            ),
            project.id,
            task_one.id,
            "waiting on vendor".into(),
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed", "claim.changed"],
        true,
    );

    // Unblock task.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .unblock_task(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(task_revision(&s.store, task_one.id).await),
            ),
            project.id,
            task_one.id,
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed"],
        false,
    );

    // Block epic with a claimed descendant: epic, task and claim all change.
    claim(&s, &worker, project.id, task_two.id, ClaimPhase::Execute).await;
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let epic_revision: i64 = sqlx::query_scalar("SELECT revision FROM epics WHERE id = ?1")
        .bind(epic.id.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    s.store
        .block_epic(
            ctx(&s.owner, s.clock.now(), Some(epic_revision)),
            project.id,
            epic.id,
            "scope under review".into(),
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["epic.changed", "task.changed", "claim.changed"],
        false,
    );

    // Unblock epic.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let epic_revision: i64 = sqlx::query_scalar("SELECT revision FROM epics WHERE id = ?1")
        .bind(epic.id.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    s.store
        .unblock_epic(
            ctx(&s.owner, s.clock.now(), Some(epic_revision)),
            project.id,
            epic.id,
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["epic.changed"],
        false,
    );

    // Cancel one task, then waive it: exclusion cascades epic completion
    // once the remaining task is done.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .cancel_task(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(task_revision(&s.store, task_one.id).await),
            ),
            project.id,
            task_one.id,
            "descoped".into(),
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed"],
        false,
    );

    let grant = claim(&s, &worker, project.id, task_two.id, ClaimPhase::Execute).await;
    let output = create_document(
        &s,
        project.id,
        DocumentOwner::Task(task_two.id),
        DocumentKind::Finding,
        "done",
    )
    .await;
    report(
        &s,
        &worker,
        project.id,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "built",
        vec![output.latest_revision_id.as_uuid()],
    )
    .await
    .unwrap();

    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .waive_task(
            ctx(
                &s.owner,
                s.clock.now(),
                Some(task_revision(&s.store, task_one.id).await),
            ),
            project.id,
            task_one.id,
            "not needed".into(),
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed", "epic.changed"],
        false,
    );

    // Cancel epic with a nonterminal descendant, on a fresh epic.
    let epic_two = create_epic(&s, project.id, goal, "E2").await;
    let task_three = create_task_full(&s, project.id, epic_two.id, "T3", false, none, none).await;
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .cancel_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic_two.id,
            "descoped".into(),
        )
        .await
        .unwrap();
    let rows = events_after(&s.store, marker).await;
    assert_group(&rows, &["epic.changed", "task.changed"], false);
    assert!(rows.iter().all(|row| row.8 == "descoped"));
    drop(task_three);

    // Explicit completion of an empty epic.
    let epic_three = create_epic(&s, project.id, goal, "E3").await;
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .complete_epic(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            epic_three.id,
            "nothing to do".into(),
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["epic.changed"],
        false,
    );
}

#[tokio::test]
async fn events_cover_dependency_document_and_selection_transitions() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic_one = create_epic(&s, project.id, goal, "E1").await;
    let epic_two = create_epic(&s, project.id, goal, "E2").await;
    let task = create_task_full(
        &s,
        project.id,
        epic_one.id,
        "T",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;

    // Dependency create and delete.
    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let dependency = s
        .store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Epic {
                dependent_id: epic_one.id,
                prerequisite_id: epic_two.id,
            },
        )
        .await
        .unwrap()
        .value;
    assert_group(
        &events_after(&s.store, marker).await,
        &["dependency.changed", "epic.changed", "epic.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .delete_dependency(
            ctx(&s.owner, s.clock.now(), Some(dependency.revision.value())),
            project.id,
            dependency.id,
        )
        .await
        .unwrap();
    assert_group(
        &events_after(&s.store, marker).await,
        &["dependency.changed", "epic.changed", "epic.changed"],
        true,
    );

    // Document create and revision append.
    let marker = last_event_id(&s.store).await;
    let document = create_document(
        &s,
        project.id,
        DocumentOwner::Task(task.id),
        DocumentKind::Plan,
        "plan v1",
    )
    .await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["document.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    let second = add_revision(&s, &s.owner, project.id, &document, 1, "plan v2").await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["document.changed"],
        true,
    );

    // Plan selection bumps the task; re-selecting the same revision is a no-op.
    let marker = last_event_id(&s.store).await;
    select_plan(&s, project.id, task.id, second.id.as_uuid()).await;
    assert_group(
        &events_after(&s.store, marker).await,
        &["task.changed"],
        true,
    );

    let marker = last_event_id(&s.store).await;
    select_plan(&s, project.id, task.id, second.id.as_uuid()).await;
    assert!(
        events_after(&s.store, marker).await.is_empty(),
        "no change, no event"
    );
}

#[tokio::test]
async fn failed_transaction_appends_no_events() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let worker = agent(&s, "worker").await;
    let grant = claim(&s, &worker, project, task.id, ClaimPhase::Execute).await;
    let output = create_document(
        &s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Finding,
        "result",
    )
    .await;

    // The prior commands' independently committed events must survive.
    let before_rows = events_after(&s.store, 0).await;
    let task_before = task_revision(&s.store, task.id).await;

    s.store.report_failpoint.store(
        failpoint::AFTER_EVENT_INSERT,
        std::sync::atomic::Ordering::Relaxed,
    );
    let err = report(
        &s,
        &worker,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "built",
        vec![output.latest_revision_id.as_uuid()],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::Storage(_)));
    s.store
        .report_failpoint
        .store(0, std::sync::atomic::Ordering::Relaxed);

    // No event rows, no session, no task/claim mutation leaked.
    assert_eq!(events_after(&s.store, 0).await, before_rows);
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    assert_eq!(task_revision(&s.store, task.id).await, task_before);
    let claim_status: String = sqlx::query_scalar("SELECT status FROM claims WHERE id = ?1")
        .bind(grant.claim.id.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    assert_eq!(claim_status, "active");

    // The same command succeeds once the fault clears, appending one group.
    report(
        &s,
        &worker,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "built",
        vec![output.latest_revision_id.as_uuid()],
    )
    .await
    .unwrap();
    assert!(event_count(&s.store).await > before_rows.len() as i64);
}

#[tokio::test]
async fn rejected_commands_leave_history_unchanged() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::None, ReviewPolicy::None).await;
    let worker = agent(&s, "worker").await;
    let before = events_after(&s.store, 0).await;

    // Revision conflict.
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(99)),
            project,
            task.id,
            shepherd_core::model::TaskPatch {
                title: Some("New".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::RevisionConflict { .. }));

    // Not eligible: execute claim while planning is required and unplanned.
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .claim_task(
            ctx(&worker, s.clock.now(), Some(1)),
            project,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotEligible(_)));

    assert_eq!(events_after(&s.store, 0).await, before);
}
