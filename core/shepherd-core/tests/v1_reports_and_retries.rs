use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::{CommandContext, Replay};
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimInput, ClaimPhase, Clock, CommandId, Epic, EpicCreate, EpicId,
    EpicStatus, GoalCreate, GoalId, Project, ProjectCreate, ProjectId, ReportInput, ReviewPolicy,
    SessionOutcome, Task, TaskCreate, TaskId, TaskStatus, TestClock,
};
use shepherd_core::storage::rows::{format_ts, insert_actor};
use shepherd_core::storage::{Store, failpoint, open, testing};
use uuid::Uuid;

async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(testing::store_options(dir.path(), "shepherd.db", clock))
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

fn ctx_with_key(
    actor: &Actor,
    now: DateTime<Utc>,
    expected_revision: Option<i64>,
    key: Uuid,
) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: key,
        expected_revision,
        now,
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
    owner: Actor,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = open_store(&dir, clock.clone()).await;
    let owner = actor(clock.now(), ActorKind::Human, "owner");
    register(&store, &owner).await;
    Setup {
        _dir: dir,
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

async fn create_goal(s: &Setup, project: ProjectId, title: &str) -> GoalId {
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
        .id
}

async fn create_epic(s: &Setup, project: ProjectId, goal: GoalId, title: &str) -> Epic {
    s.clock.advance(TimeDelta::milliseconds(2));
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

async fn create_task_full(
    s: &Setup,
    project: ProjectId,
    epic: EpicId,
    title: &str,
    planning_required: bool,
    plan_review: ReviewPolicy,
    work_review: ReviewPolicy,
) -> Task {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project,
            epic,
            TaskCreate {
                title: title.to_string(),
                type_key: "code".to_string(),
                planning_required: Some(planning_required),
                plan_review: Some(plan_review),
                work_review: Some(work_review),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

async fn claim_and_report_setup(
    s: &Setup,
    work_review: ReviewPolicy,
) -> (ProjectId, EpicId, Task, Uuid, String, String) {
    let project = create_project(s, "P").await;
    let goal = create_goal(s, project.id, "G").await;
    let epic = create_epic(s, project.id, goal, "E").await;
    let task = create_task_full(
        s,
        project.id,
        epic.id,
        "T",
        false,
        ReviewPolicy::None,
        work_review,
    )
    .await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();
    let (doc_id, rev_id) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "finding",
        &format_ts(&s.clock.now()),
    )
    .await;
    let _ = doc_id;
    (
        project.id,
        epic.id,
        task,
        grant.claim.id,
        grant.lease_token,
        rev_id,
    )
}

fn success_input(rev_id: &str) -> ReportInput {
    ReportInput {
        outcome: SessionOutcome::Succeeded,
        summary: "done".into(),
        failure_reason: String::new(),
        document_revision_ids: vec![rev_id.parse().unwrap()],
        links: serde_json::json!([]),
        selected_plan_revision_id: None,
    }
}

fn failed_input() -> ReportInput {
    ReportInput {
        outcome: SessionOutcome::Failed,
        summary: String::new(),
        failure_reason: "something went wrong".into(),
        document_revision_ids: Vec::new(),
        links: serde_json::json!([]),
        selected_plan_revision_id: None,
    }
}

async fn reload_task(s: &Setup, project: ProjectId, task_id: TaskId) -> Task {
    s.store.get_task(&project, &task_id).await.unwrap()
}

async fn reload_epic(s: &Setup, project: ProjectId, epic_id: EpicId) -> Epic {
    s.store.get_epic(&project, &epic_id).await.unwrap()
}

async fn session_count(store: &Store, task_id: TaskId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM sessions WHERE task_id = ?1")
        .bind(task_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn submission_count(store: &Store, task_id: TaskId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM submissions WHERE task_id = ?1")
        .bind(task_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn claim_status(store: &Store, claim_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM claims WHERE id = ?1")
        .bind(claim_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn submission_row(store: &Store, task_id: TaskId) -> (String, String, String) {
    sqlx::query_as("SELECT status, kind, policy FROM submissions WHERE task_id = ?1 LIMIT 1")
        .bind(task_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

// --- Rollback tests ---

async fn assert_rollback(s: &Setup, project: ProjectId, task: &Task, claim_id: Uuid) {
    assert_eq!(claim_status(&s.store, claim_id).await, "active");
    assert_eq!(session_count(&s.store, task.id).await, 0);
    assert_eq!(submission_count(&s.store, task.id).await, 0);
    let reloaded = reload_task(s, project, task.id).await;
    assert_eq!(reloaded.attempt_count, task.attempt_count);
    assert_eq!(reloaded.phase, task.phase);
    assert_eq!(reloaded.status, task.status);
}

#[tokio::test]
async fn failure_after_lease_close_rolls_back_everything() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;
    let task = reload_task(&s, project, task.id).await;

    s.store.report_failpoint.store(
        failpoint::AFTER_LEASE_CLOSE,
        std::sync::atomic::Ordering::Relaxed,
    );
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Storage(_)));
    assert_rollback(&s, project, &task, claim_id).await;
}

#[tokio::test]
async fn failure_after_session_insert_rolls_back_everything() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;
    let task = reload_task(&s, project, task.id).await;

    s.store.report_failpoint.store(
        failpoint::AFTER_SESSION_INSERT,
        std::sync::atomic::Ordering::Relaxed,
    );
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Storage(_)));
    assert_rollback(&s, project, &task, claim_id).await;
}

#[tokio::test]
async fn failure_after_task_update_rolls_back_everything() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;
    let task = reload_task(&s, project, task.id).await;

    s.store.report_failpoint.store(
        failpoint::AFTER_TASK_UPDATE,
        std::sync::atomic::Ordering::Relaxed,
    );
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Storage(_)));
    assert_rollback(&s, project, &task, claim_id).await;
}

#[tokio::test]
async fn failure_after_event_insert_rolls_back_everything() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;
    let task = reload_task(&s, project, task.id).await;

    s.store.report_failpoint.store(
        failpoint::AFTER_EVENT_INSERT,
        std::sync::atomic::Ordering::Relaxed,
    );
    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Storage(_)));
    assert_rollback(&s, project, &task, claim_id).await;
}

// --- Idempotency tests ---

#[tokio::test]
async fn same_key_successful_retry_returns_original_session() {
    let s = setup().await;
    let (project, _, _task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;

    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    s.clock.advance(TimeDelta::milliseconds(2));
    let first = s
        .store
        .report_claim(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap();
    assert!(matches!(&first, Replay::Fresh(_)));
    let session = first.into_inner();

    s.clock.advance(TimeDelta::milliseconds(2));
    let second = s
        .store
        .report_claim(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap();
    assert!(second.is_replay());
    assert_eq!(second.into_inner().id, session.id);
    assert_eq!(session_count(&s.store, _task.id).await, 1);
}

#[tokio::test]
async fn different_payload_on_same_key_returns_idempotency_conflict() {
    let s = setup().await;
    let (project, _, _task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;

    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "hash-1",
        )
        .await
        .unwrap();

    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "hash-2",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::IdempotencyConflict));
}

// --- Lease validation tests ---

#[tokio::test]
async fn expired_claim_report_fails_with_lease_invalid() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;

    // Advance past the default 300s TTL.
    s.clock.advance(TimeDelta::seconds(301));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::LeaseInvalid));
    // Transaction rolled back so claim keeps its DB status; it is logically
    // expired but the reconciliation UPDATE was part of the rolled-back tx.
    assert_eq!(claim_status(&s.store, claim_id).await, "active");
    assert_eq!(session_count(&s.store, task.id).await, 0);
}

#[tokio::test]
async fn revoked_claim_report_fails_with_lease_invalid() {
    let s = setup().await;
    let (project, epic_id, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;

    // Block the epic, which revokes all descendant claims.
    s.clock.advance(TimeDelta::milliseconds(2));
    let epic = reload_epic(&s, project, epic_id).await;
    s.store
        .block_epic(
            ctx(&s.owner, s.clock.now(), Some(epic.revision.value())),
            project,
            epic_id,
            "needs rework".to_string(),
        )
        .await
        .unwrap();

    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::LeaseInvalid));
    assert_eq!(claim_status(&s.store, claim_id).await, "revoked");
    assert_eq!(session_count(&s.store, task.id).await, 0);
}

#[tokio::test]
async fn wrong_actor_report_fails_with_lease_invalid() {
    let s = setup().await;
    let (project, _, task, claim_id, token, rev_id) =
        claim_and_report_setup(&s, ReviewPolicy::None).await;

    let other = actor(s.clock.now(), ActorKind::Agent, "intruder");
    register(&s.store, &other).await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let err = s
        .store
        .report_claim(
            ctx(&other, s.clock.now(), None),
            project,
            claim_id,
            &token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::LeaseInvalid));
    assert_eq!(session_count(&s.store, task.id).await, 0);
}

// --- Attempt counting ---

#[tokio::test]
async fn execution_attempts_increment_once_planning_does_not() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let task = create_task_full(
        &s,
        project.id,
        epic.id,
        "T",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let now_text = format_ts(&s.clock.now());
    let (_, plan_rev) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "plan",
        &now_text,
    )
    .await;
    let plan_rev_uuid: Uuid = plan_rev.parse().unwrap();

    // Failed plan report: attempt_count must NOT increment.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-plan-1",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            failed_input(),
            "report-plan-fail",
        )
        .await
        .unwrap();
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(
        t.attempt_count, 0,
        "plan failure must not increment attempt_count"
    );

    // Successful plan report: still no increment.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(t.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-plan-2",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "planned".into(),
                failure_reason: String::new(),
                document_revision_ids: vec![plan_rev_uuid],
                links: serde_json::json!([]),
                selected_plan_revision_id: Some(plan_rev_uuid),
            },
            "report-plan-ok",
        )
        .await
        .unwrap();
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(
        t.attempt_count, 0,
        "plan success must not increment attempt_count"
    );

    // Now task is in execution phase. Seed a work document.
    let (_, work_rev) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "finding",
        &format_ts(&s.clock.now()),
    )
    .await;

    // Failed execute report: attempt_count increments to 1.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(t.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-exec-1",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            failed_input(),
            "report-exec-fail",
        )
        .await
        .unwrap();
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(
        t.attempt_count, 1,
        "execute failure must increment attempt_count"
    );

    // Successful execute report: attempt_count increments to 2.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(t.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-exec-2",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            success_input(&work_rev),
            "report-exec-ok",
        )
        .await
        .unwrap();
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(
        t.attempt_count, 2,
        "execute success must increment attempt_count"
    );
    assert_eq!(t.status, TaskStatus::Done);
}

// --- Phase transition tests ---

#[tokio::test]
async fn successful_plan_report_with_none_policy_advances_to_execution() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let task = create_task_full(
        &s,
        project.id,
        epic.id,
        "T",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let (_, rev_id) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "plan",
        &format_ts(&s.clock.now()),
    )
    .await;
    let rev_uuid: Uuid = rev_id.parse().unwrap();

    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    let session = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "planned".into(),
                failure_reason: String::new(),
                document_revision_ids: vec![rev_uuid],
                links: serde_json::json!([]),
                selected_plan_revision_id: Some(rev_uuid),
            },
            "report-hash",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(session.outcome, SessionOutcome::Succeeded);
    assert_eq!(session.phase, ClaimPhase::Plan);
    assert_eq!(claim_status(&s.store, grant.claim.id).await, "reported");

    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(t.phase, shepherd_core::model::TaskPhase::Execution);
    assert_eq!(t.status, TaskStatus::Active);

    let (sub_status, sub_kind, sub_policy) = submission_row(&s.store, task.id).await;
    assert_eq!(sub_status, "accepted");
    assert_eq!(sub_kind, "plan");
    assert_eq!(sub_policy, "none");

    // Verify selected_plan_revision_id and accepted_plan_submission_id are set.
    let (selected, accepted): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT selected_plan_revision_id, accepted_plan_submission_id FROM tasks WHERE id = ?1",
    )
    .bind(task.id.to_string())
    .fetch_one(s.store.pool())
    .await
    .unwrap();
    assert_eq!(selected.as_deref(), Some(rev_id.as_str()));
    assert!(accepted.is_some());
}

#[tokio::test]
async fn successful_plan_report_with_human_policy_advances_to_plan_review() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let task = create_task_full(
        &s,
        project.id,
        epic.id,
        "T",
        true,
        ReviewPolicy::Human,
        ReviewPolicy::Human,
    )
    .await;
    let (_, rev_id) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "plan",
        &format_ts(&s.clock.now()),
    )
    .await;
    let rev_uuid: Uuid = rev_id.parse().unwrap();

    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "planned".into(),
                failure_reason: String::new(),
                document_revision_ids: vec![rev_uuid],
                links: serde_json::json!([]),
                selected_plan_revision_id: Some(rev_uuid),
            },
            "report-hash",
        )
        .await
        .unwrap();

    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(t.phase, shepherd_core::model::TaskPhase::PlanReview);
    assert_eq!(t.status, TaskStatus::Active);

    let (sub_status, sub_kind, sub_policy) = submission_row(&s.store, task.id).await;
    assert_eq!(sub_status, "pending");
    assert_eq!(sub_kind, "plan");
    assert_eq!(sub_policy, "human");

    // selected_plan_revision_id set, but accepted_plan_submission_id is NOT set.
    let (selected, accepted): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT selected_plan_revision_id, accepted_plan_submission_id FROM tasks WHERE id = ?1",
    )
    .bind(task.id.to_string())
    .fetch_one(s.store.pool())
    .await
    .unwrap();
    assert_eq!(selected.as_deref(), Some(rev_id.as_str()));
    assert!(accepted.is_none());
}

#[tokio::test]
async fn successful_execute_report_with_none_policy_completes_and_cascades() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic_a = create_epic(&s, project.id, goal, "A").await;
    let epic_b = create_epic(&s, project.id, goal, "B").await;
    let task_a = create_task_full(
        &s,
        project.id,
        epic_a.id,
        "TA",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;

    // Complete task A (and epic A) via the test helper.
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .complete_task_for_test(
            ctx(&s.owner, s.clock.now(), Some(task_a.revision.value())),
            project.id,
            task_a.id,
        )
        .await
        .unwrap();

    // Add dependency: epic B depends on epic A.
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            shepherd_core::model::DependencyCreate::Epic {
                dependent_id: epic_b.id,
                prerequisite_id: epic_a.id,
            },
        )
        .await
        .unwrap();

    // Create task in epic B and claim it.
    let task_b = create_task_full(
        &s,
        project.id,
        epic_b.id,
        "TB",
        false,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let (_, rev_id) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task_b.id,
        s.owner.id,
        "finding",
        &format_ts(&s.clock.now()),
    )
    .await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_b.revision.value())),
            project.id,
            task_b.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::milliseconds(2));
    let session = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            success_input(&rev_id),
            "report-hash",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(session.outcome, SessionOutcome::Succeeded);

    let t = reload_task(&s, project.id, task_b.id).await;
    assert_eq!(t.status, TaskStatus::Done);
    assert_eq!(t.phase, shepherd_core::model::TaskPhase::Complete);
    assert_eq!(t.attempt_count, 1);

    // Epic B auto-completed via cascade.
    let eb = reload_epic(&s, project.id, epic_b.id).await;
    assert_eq!(eb.status, EpicStatus::Done);

    let (sub_status, sub_kind, _) = submission_row(&s.store, task_b.id).await;
    assert_eq!(sub_status, "accepted");
    assert_eq!(sub_kind, "work");
}

#[tokio::test]
async fn failed_reports_leave_phase_unchanged() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal, "E").await;
    let task = create_task_full(
        &s,
        project.id,
        epic.id,
        "T",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    let (_, plan_rev) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "plan",
        &format_ts(&s.clock.now()),
    )
    .await;
    let plan_rev_uuid: Uuid = plan_rev.parse().unwrap();

    // Failed plan report: phase stays at planning.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-plan",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    let session = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            failed_input(),
            "report-plan-fail",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(session.outcome, SessionOutcome::Failed);
    assert_eq!(session.phase, ClaimPhase::Plan);
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(t.phase, shepherd_core::model::TaskPhase::Planning);
    assert_eq!(t.status, TaskStatus::Active);
    assert_eq!(submission_count(&s.store, task.id).await, 0);
    assert_eq!(session_count(&s.store, task.id).await, 1);

    // Advance to execution via successful plan report.
    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(t.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-plan-2",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "done".into(),
                failure_reason: String::new(),
                document_revision_ids: vec![plan_rev_uuid],
                links: serde_json::json!([]),
                selected_plan_revision_id: Some(plan_rev_uuid),
            },
            "report-plan-ok",
        )
        .await
        .unwrap();

    // Failed execute report: phase stays at execution.
    let t = reload_task(&s, project.id, task.id).await;
    let (_, work_rev) = testing::seed_task_document(
        s.store.pool(),
        project.id,
        task.id,
        s.owner.id,
        "finding",
        &format_ts(&s.clock.now()),
    )
    .await;
    let _ = work_rev;

    s.clock.advance(TimeDelta::milliseconds(2));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(t.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Execute,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-exec",
        )
        .await
        .unwrap()
        .into_inner();
    s.clock.advance(TimeDelta::milliseconds(2));
    let session = s
        .store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            failed_input(),
            "report-exec-fail",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(session.outcome, SessionOutcome::Failed);
    assert_eq!(session.phase, ClaimPhase::Execute);
    let t = reload_task(&s, project.id, task.id).await;
    assert_eq!(t.phase, shepherd_core::model::TaskPhase::Execution);
    assert_eq!(t.attempt_count, 1);
}
