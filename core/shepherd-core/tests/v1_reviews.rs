use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimGrant, ClaimInput, ClaimPhase, Clock, CommandId, Document,
    DocumentKind, Epic, EpicCreate, EpicId, GoalCreate, GoalId, Project, ProjectCreate, ProjectId,
    ReportInput, Review, ReviewDecision, ReviewInput, ReviewPolicy, SelectPlanInput,
    SessionOutcome, Task, TaskCreate, TaskId, TestClock,
};
use shepherd_core::storage::rows::insert_actor;
use shepherd_core::storage::{Store, open, testing};
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
    dir: tempfile::TempDir,
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
        dir,
        clock,
        store,
        owner,
    }
}

async fn agent(s: &Setup, label: &str) -> Actor {
    s.clock.advance(TimeDelta::milliseconds(2));
    let agent = actor(s.clock.now(), ActorKind::Agent, label);
    register(&s.store, &agent).await;
    agent
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

/// Fresh project -> goal -> epic -> task chain for one test.
async fn scaffold(
    s: &Setup,
    planning_required: bool,
    plan_review: ReviewPolicy,
    work_review: ReviewPolicy,
) -> (ProjectId, EpicId, Task) {
    let project = create_project(s, "P").await;
    let goal = create_goal(s, project.id, "G").await;
    let epic = create_epic(s, project.id, goal, "E").await;
    let task = create_task_full(
        s,
        project.id,
        epic.id,
        "T",
        planning_required,
        plan_review,
        work_review,
    )
    .await;
    (project.id, epic.id, task)
}

async fn create_doc(s: &Setup, project: ProjectId, task: TaskId, kind: DocumentKind) -> Document {
    s.clock.advance(TimeDelta::milliseconds(2));
    testing::create_task_document(&s.store, &s.owner, project, task, kind).await
}

async fn try_claim(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    task: TaskId,
    phase: ClaimPhase,
    submission_id: Option<Uuid>,
) -> Result<ClaimGrant, DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, task).await;
    s.store
        .claim_task(
            ctx(who, s.clock.now(), Some(revision)),
            project,
            task,
            ClaimInput {
                phase,
                submission_id,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .map(shepherd_core::commands::Replay::into_inner)
}

async fn claim(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    task: TaskId,
    phase: ClaimPhase,
    submission_id: Option<Uuid>,
) -> (Uuid, String) {
    let grant = try_claim(s, who, project, task, phase, submission_id)
        .await
        .unwrap();
    (grant.claim.id, grant.lease_token)
}

async fn report_success(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    claim_id: Uuid,
    token: &str,
    revisions: Vec<Uuid>,
) {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(who, s.clock.now(), None),
            project,
            claim_id,
            token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "done".into(),
                failure_reason: String::new(),
                document_revision_ids: revisions,
                links: serde_json::json!([]),
            },
            "report-hash",
        )
        .await
        .unwrap();
}

struct ReviewFixture {
    project: ProjectId,
    task: TaskId,
    submission_id: Uuid,
    producer: Actor,
}

/// Public plan flow up to a plan submission: producer agent claims plan, the
/// owner uploads and selects the plan document, producer reports success.
async fn pending_plan_submission(s: &Setup, plan_review: ReviewPolicy) -> ReviewFixture {
    let (project, _, task) = scaffold(s, true, plan_review, ReviewPolicy::None).await;
    let producer = agent(s, "producer").await;
    let (claim_id, token) = claim(s, &producer, project, task.id, ClaimPhase::Plan, None).await;
    let document = create_doc(s, project, task.id, DocumentKind::Plan).await;
    let revision = document.latest_revision_id.as_uuid();
    select_plan(s, project, task.id, revision).await;
    report_success(s, &producer, project, claim_id, &token, vec![revision]).await;
    ReviewFixture {
        project,
        task: task.id,
        submission_id: pending_submission_id(&s.store, task.id).await,
        producer,
    }
}

/// Public execute flow up to a work submission: producer agent claims execute,
/// reports success citing a task-owned output revision.
async fn pending_work_submission(s: &Setup, work_review: ReviewPolicy) -> ReviewFixture {
    let (project, _, task) = scaffold(s, false, ReviewPolicy::None, work_review).await;
    let producer = agent(s, "producer").await;
    let (claim_id, token) = claim(s, &producer, project, task.id, ClaimPhase::Execute, None).await;
    let document = create_doc(s, project, task.id, DocumentKind::Finding).await;
    let revision = document.latest_revision_id.as_uuid();
    report_success(s, &producer, project, claim_id, &token, vec![revision]).await;
    ReviewFixture {
        project,
        task: task.id,
        submission_id: pending_submission_id(&s.store, task.id).await,
        producer,
    }
}

async fn select_plan(s: &Setup, project: ProjectId, task: TaskId, revision_id: Uuid) {
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, task).await;
    s.store
        .select_task_plan(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            project,
            task,
            SelectPlanInput {
                document_revision_id: revision_id,
            },
        )
        .await
        .unwrap();
}

// --- Persisted-state probes ---

async fn task_revision(store: &Store, task: TaskId) -> i64 {
    sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
        .bind(task.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// (status, phase, revision, selected_plan_revision_id, accepted_plan_submission_id)
async fn task_state(
    store: &Store,
    task: TaskId,
) -> (String, String, i64, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT status, phase, revision, selected_plan_revision_id, \
         accepted_plan_submission_id FROM tasks WHERE id = ?1",
    )
    .bind(task.to_string())
    .fetch_one(store.pool())
    .await
    .unwrap()
}

async fn pending_submission_id(store: &Store, task: TaskId) -> Uuid {
    let id: String =
        sqlx::query_scalar("SELECT id FROM submissions WHERE task_id = ?1 AND status = 'pending'")
            .bind(task.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    Uuid::parse_str(&id).unwrap()
}

/// (status, close_reason, submission_id)
async fn claim_state(store: &Store, claim: Uuid) -> (String, String, Option<String>) {
    sqlx::query_as("SELECT status, close_reason, submission_id FROM claims WHERE id = ?1")
        .bind(claim.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn event_type_count(store: &Store, event_type: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type = ?1")
        .bind(event_type)
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// (status, revision, withdraw_reason)
async fn submission_state(store: &Store, submission: Uuid) -> (String, i64, String) {
    sqlx::query_as("SELECT status, revision, withdraw_reason FROM submissions WHERE id = ?1")
        .bind(submission.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn reviews_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM reviews")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn event_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// (decision, actor_id, reason) of the single review for a submission.
async fn review_row(store: &Store, submission: Uuid) -> (String, String, String) {
    sqlx::query_as("SELECT decision, actor_id, reason FROM reviews WHERE submission_id = ?1")
        .bind(submission.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn epic_status(store: &Store, task: TaskId) -> String {
    sqlx::query_scalar(
        "SELECT e.status FROM epics e JOIN tasks t ON t.epic_id = e.id WHERE t.id = ?1",
    )
    .bind(task.to_string())
    .fetch_one(store.pool())
    .await
    .unwrap()
}

/// Full persisted snapshot asserted unchanged after a failed mutation.
async fn mutation_snapshot(
    store: &Store,
    task: TaskId,
    submission: Uuid,
) -> (
    (String, String, i64, Option<String>, Option<String>),
    (String, i64, String),
    i64,
    i64,
) {
    (
        task_state(store, task).await,
        submission_state(store, submission).await,
        reviews_count(store).await,
        event_count(store).await,
    )
}

#[allow(clippy::too_many_arguments)]
async fn try_review(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    submission: Uuid,
    expected_revision: Option<i64>,
    decision: ReviewDecision,
    reason: &str,
    claim: Option<(Uuid, &str)>,
) -> Result<Review, DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    let (claim_id, token) = match claim {
        Some((id, token)) => (Some(id), Some(token)),
        None => (None, None),
    };
    s.store
        .review_submission(
            ctx(who, s.clock.now(), expected_revision),
            project,
            submission,
            token,
            ReviewInput {
                decision,
                reason: reason.to_string(),
                claim_id,
            },
            "review-hash",
        )
        .await
        .map(shepherd_core::commands::Replay::into_inner)
}

/// Plan accepted under the none policy, then a pending human work submission:
/// the shape for "rejected work returns execution with plan retained".
async fn work_pending_with_plan(s: &Setup) -> ReviewFixture {
    let (project, _, task) = scaffold(s, true, ReviewPolicy::None, ReviewPolicy::Human).await;
    let producer = agent(s, "producer").await;
    let (claim_id, token) = claim(s, &producer, project, task.id, ClaimPhase::Plan, None).await;
    let document = create_doc(s, project, task.id, DocumentKind::Plan).await;
    let revision = document.latest_revision_id.as_uuid();
    select_plan(s, project, task.id, revision).await;
    report_success(s, &producer, project, claim_id, &token, vec![revision]).await;
    let (claim_id, token) = claim(s, &producer, project, task.id, ClaimPhase::Execute, None).await;
    let output = create_doc(s, project, task.id, DocumentKind::Finding).await;
    report_success(
        s,
        &producer,
        project,
        claim_id,
        &token,
        vec![output.latest_revision_id.as_uuid()],
    )
    .await;
    ReviewFixture {
        project,
        task: task.id,
        submission_id: pending_submission_id(&s.store, task.id).await,
        producer,
    }
}

// --- Acceptance: agent token cannot approve human gate ---

#[tokio::test]
async fn agent_token_cannot_approve_human_gate() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let reviewer = agent(&s, "reviewer").await;

    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::Forbidden(_)),
        "expected forbidden, got: {err:?}"
    );
    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );
}

#[tokio::test]
async fn owner_cannot_decide_agent_policy_review() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;

    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;
    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::Forbidden(_)),
        "expected forbidden for the owner on an agent gate, got: {err:?}"
    );
    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );
}

// --- Acceptance: rejected plan returns planning ---

#[tokio::test]
async fn rejected_plan_returns_planning_and_clears_selection_and_acceptance() {
    let s = setup().await;
    let f = pending_plan_submission(&s, ReviewPolicy::Human).await;

    let review = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        "plan is incomplete",
        None,
    )
    .await
    .unwrap();

    let (status, phase, _, selected, accepted) = task_state(&s.store, f.task).await;
    assert_eq!(status, "active");
    assert_eq!(phase, "planning");
    assert_eq!(selected, None, "rejection must clear the stale selection");
    assert_eq!(accepted, None, "rejection must clear the acceptance");
    let (sub_status, sub_revision, _) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "rejected");
    assert_eq!(sub_revision, 2);
    let (decision, actor_id, reason) = review_row(&s.store, f.submission_id).await;
    assert_eq!(decision, "reject");
    assert_eq!(actor_id, s.owner.id.to_string());
    assert_eq!(reason, "plan is incomplete");
    assert_eq!(review.reason, "plan is incomplete");
    assert_eq!(event_type_count(&s.store, "review.recorded").await, 1);
}

#[tokio::test]
async fn approved_plan_moves_to_execution_and_pins_acceptance() {
    let s = setup().await;
    let f = pending_plan_submission(&s, ReviewPolicy::Human).await;

    try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap();

    let (status, phase, _, selected, accepted) = task_state(&s.store, f.task).await;
    assert_eq!(status, "active");
    assert_eq!(phase, "execution");
    assert!(selected.is_some(), "approval keeps the selected plan");
    assert_eq!(accepted, Some(f.submission_id.to_string()));
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "accepted");
}

// --- Acceptance: rejected work returns execution with plan retained ---

#[tokio::test]
async fn rejected_work_returns_execution_with_plan_retained() {
    let s = setup().await;
    let f = work_pending_with_plan(&s).await;
    let (_, _, _, selected_before, accepted_before) = task_state(&s.store, f.task).await;
    assert!(selected_before.is_some());
    assert!(accepted_before.is_some());

    try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        "output is wrong",
        None,
    )
    .await
    .unwrap();

    let (status, phase, _, selected, accepted) = task_state(&s.store, f.task).await;
    assert_eq!(status, "active");
    assert_eq!(phase, "execution");
    assert_eq!(selected, selected_before, "plan selection is retained");
    assert_eq!(accepted, accepted_before, "plan acceptance is retained");
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "rejected");
}

// --- Approve work: completion cascade ---

#[tokio::test]
async fn approved_work_completes_task_and_cascades() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;

    try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap();

    let (status, phase, ..) = task_state(&s.store, f.task).await;
    assert_eq!(status, "done");
    assert_eq!(phase, "complete");
    assert_eq!(epic_status(&s.store, f.task).await, "done");
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "accepted");
    assert_eq!(event_type_count(&s.store, "review.recorded").await, 1);
}

// --- Agent review decision rides the claim ---

#[tokio::test]
async fn agent_review_decision_releases_claim() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    let claims_before = event_type_count(&s.store, "claim.changed").await;
    let submissions_before = event_type_count(&s.store, "submission.changed").await;
    let tasks_before = event_type_count(&s.store, "task.changed").await;

    try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((claim_id, &token)),
    )
    .await
    .unwrap();

    let (status, phase, ..) = task_state(&s.store, f.task).await;
    assert_eq!(status, "done");
    assert_eq!(phase, "complete");
    let (claim_status, ..) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "released");
    let (decision, actor_id, _) = review_row(&s.store, f.submission_id).await;
    assert_eq!(decision, "approve");
    assert_eq!(actor_id, reviewer.id.to_string());
    // plan/08: the decision emits review.recorded plus one submission,
    // claim (release) and task (completion) event.
    assert_eq!(event_type_count(&s.store, "review.recorded").await, 1);
    assert_eq!(
        event_type_count(&s.store, "claim.changed").await,
        claims_before + 1
    );
    assert_eq!(
        event_type_count(&s.store, "submission.changed").await,
        submissions_before + 1
    );
    assert_eq!(
        event_type_count(&s.store, "task.changed").await,
        tasks_before + 1
    );
}

#[tokio::test]
async fn agent_decision_requires_its_own_valid_lease() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, _token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;

    // Wrong lease token.
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((claim_id, "forged-token")),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::LeaseInvalid),
        "expected lease_invalid, got: {err:?}"
    );

    // Missing claim reference.
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::Validation {
                field: "claim_id",
                ..
            }
        ),
        "expected claim_id validation, got: {err:?}"
    );

    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );
}

// --- Failure modes leave persisted state unchanged ---

#[tokio::test]
async fn stale_revision_and_blank_reason_leave_state_unchanged() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;

    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(7),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::RevisionConflict {
                expected: 7,
                actual: 1
            }
        ),
        "expected revision conflict, got: {err:?}"
    );

    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        "   ",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ),
        "expected blank-reason validation, got: {err:?}"
    );

    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        &"x".repeat(2_001),
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ),
        "expected oversize-reason validation, got: {err:?}"
    );

    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );
}

#[tokio::test]
async fn decision_on_decided_submission_is_superseded() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        "redo",
        None,
    )
    .await
    .unwrap();

    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(2),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::SubmissionSuperseded(_)),
        "expected submission_superseded, got: {err:?}"
    );
    assert_eq!(reviews_count(&s.store).await, 1);
}

#[tokio::test]
async fn review_decision_replays_idempotently() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));

    s.clock.advance(TimeDelta::milliseconds(2));
    let first = s
        .store
        .review_submission(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), key),
            f.project,
            f.submission_id,
            None,
            ReviewInput {
                decision: ReviewDecision::Approve,
                reason: String::new(),
                claim_id: None,
            },
            "replay-hash",
        )
        .await
        .unwrap();
    assert!(!first.is_replay());
    let first = first.into_inner();

    // Same key and request: replayed result even though the revision is stale.
    s.clock.advance(TimeDelta::milliseconds(2));
    let second = s
        .store
        .review_submission(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), key),
            f.project,
            f.submission_id,
            None,
            ReviewInput {
                decision: ReviewDecision::Approve,
                reason: String::new(),
                claim_id: None,
            },
            "replay-hash",
        )
        .await
        .unwrap();
    assert!(second.is_replay());
    assert_eq!(second.into_inner().id, first.id);
    assert_eq!(reviews_count(&s.store).await, 1);
}

async fn try_withdraw(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    submission: Uuid,
    expected_revision: Option<i64>,
    reason: &str,
) -> Result<shepherd_core::model::Submission, DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .withdraw_submission(
            ctx(who, s.clock.now(), expected_revision),
            project,
            submission,
            reason.to_string(),
            "withdraw-hash",
        )
        .await
        .map(shepherd_core::commands::Replay::into_inner)
}

// --- Acceptance: withdrawal revokes review claim; late review fails ---

#[tokio::test]
async fn withdrawal_revokes_review_claim_and_late_review_fails() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    let claims_before = event_type_count(&s.store, "claim.changed").await;
    let submissions_before = event_type_count(&s.store, "submission.changed").await;
    let tasks_before = event_type_count(&s.store, "task.changed").await;

    let withdrawn = try_withdraw(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        "owner takes it back",
    )
    .await
    .unwrap();
    assert_eq!(withdrawn.revision.value(), 2);

    let (sub_status, sub_revision, withdraw_reason) =
        submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "withdrawn");
    assert_eq!(sub_revision, 2);
    assert_eq!(withdraw_reason, "owner takes it back");
    let (claim_status, close_reason, _) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "revoked");
    assert_eq!(close_reason, "owner takes it back");
    let (_, phase, ..) = task_state(&s.store, f.task).await;
    assert_eq!(phase, "execution");
    assert_eq!(reviews_count(&s.store).await, 0, "no fabricated review");
    // plan/08: withdrawal emits one submission, claim (revoke) and task
    // event, and no review.recorded.
    assert_eq!(event_type_count(&s.store, "review.recorded").await, 0);
    assert_eq!(
        event_type_count(&s.store, "claim.changed").await,
        claims_before + 1
    );
    assert_eq!(
        event_type_count(&s.store, "submission.changed").await,
        submissions_before + 1
    );
    assert_eq!(
        event_type_count(&s.store, "task.changed").await,
        tasks_before + 1
    );

    // Late review: the revoked claim no longer authorizes a decision.
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(2),
        ReviewDecision::Approve,
        "",
        Some((claim_id, &token)),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::LeaseInvalid),
        "expected lease_invalid after withdrawal, got: {err:?}"
    );
}

#[tokio::test]
async fn withdrawn_plan_returns_planning_with_selection_retained() {
    let s = setup().await;
    let f = pending_plan_submission(&s, ReviewPolicy::Human).await;
    let (_, _, _, selected_before, _) = task_state(&s.store, f.task).await;
    assert!(selected_before.is_some());

    // The producer may withdraw its own submission.
    try_withdraw(
        &s,
        &f.producer,
        f.project,
        f.submission_id,
        Some(1),
        "found a gap",
    )
    .await
    .unwrap();

    let (_, phase, _, selected, accepted) = task_state(&s.store, f.task).await;
    assert_eq!(phase, "planning");
    assert_eq!(selected, selected_before, "withdrawal keeps the selection");
    assert_eq!(accepted, None);
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "withdrawn");
}

#[tokio::test]
async fn unrelated_agent_cannot_withdraw() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let bystander = agent(&s, "bystander").await;

    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;
    let err = try_withdraw(&s, &bystander, f.project, f.submission_id, Some(1), "mine").await;
    assert!(
        matches!(err, Err(DomainError::Forbidden(_))),
        "expected forbidden, got: {err:?}"
    );
    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );
}

#[tokio::test]
async fn withdraw_requires_reason_and_fresh_revision() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let before = mutation_snapshot(&s.store, f.task, f.submission_id).await;

    let err = try_withdraw(&s, &s.owner, f.project, f.submission_id, Some(1), "  ")
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ),
        "expected blank-reason validation, got: {err:?}"
    );

    let err = try_withdraw(&s, &s.owner, f.project, f.submission_id, Some(5), "stale")
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::RevisionConflict {
                expected: 5,
                actual: 1
            }
        ),
        "expected revision conflict, got: {err:?}"
    );
    assert_eq!(
        mutation_snapshot(&s.store, f.task, f.submission_id).await,
        before
    );

    try_withdraw(&s, &s.owner, f.project, f.submission_id, Some(1), "redo")
        .await
        .unwrap();
    let err = try_withdraw(&s, &s.owner, f.project, f.submission_id, Some(2), "again")
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::SubmissionSuperseded(_)),
        "expected submission_superseded, got: {err:?}"
    );
}

#[tokio::test]
async fn withdraw_then_policy_change_is_the_documented_escape() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;

    try_withdraw(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        "no reviewer agent available",
    )
    .await
    .unwrap();

    // Idle after withdrawal: the owner may lower the gate to human review.
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, f.task).await;
    s.store
        .update_task(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            f.project,
            f.task,
            shepherd_core::model::TaskPatch {
                title: None,
                description: None,
                type_key: None,
                planning_required: None,
                plan_review: None,
                work_review: Some(ReviewPolicy::Human),
            },
        )
        .await
        .unwrap();

    let policy: String = sqlx::query_scalar("SELECT work_review FROM tasks WHERE id = ?1")
        .bind(f.task.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    assert_eq!(policy, "human");
}

#[tokio::test]
async fn concurrent_decision_and_withdrawal_have_one_winner() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let store_b = open_store(&s.dir, s.clock.clone()).await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let (approve, withdraw) = tokio::join!(
        s.store.review_submission(
            ctx(&s.owner, s.clock.now(), Some(1)),
            f.project,
            f.submission_id,
            None,
            ReviewInput {
                decision: ReviewDecision::Approve,
                reason: String::new(),
                claim_id: None,
            },
            "race-approve",
        ),
        store_b.withdraw_submission(
            ctx(&s.owner, s.clock.now(), Some(1)),
            f.project,
            f.submission_id,
            "changed my mind".to_string(),
            "race-withdraw",
        ),
    );

    let winners = [approve.is_ok(), withdraw.is_ok()];
    assert_eq!(
        winners.iter().filter(|ok| **ok).count(),
        1,
        "exactly one mutation must win: {approve:?} vs {withdraw:?}"
    );
    let (sub_status, sub_revision, _) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_revision, 2);
    if approve.is_ok() {
        assert_eq!(sub_status, "accepted");
        assert_eq!(reviews_count(&s.store).await, 1);
    } else {
        assert_eq!(sub_status, "withdrawn");
        assert_eq!(reviews_count(&s.store).await, 0);
    }
}

// --- Acceptance: producer cannot agent-review own output using another session ---

#[tokio::test]
async fn producer_cannot_agent_review_own_output_using_another_session() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;

    // A fresh claim is a fresh session; the credential is what matters.
    let err = try_claim(
        &s,
        &f.producer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::ProducerCannotReview),
        "expected producer_cannot_review, got: {err:?}"
    );

    // Credential separation (plan/15 D06): a separately registered agent may.
    let reviewer = agent(&s, "reviewer").await;
    let grant = try_claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap();
    let (status, _, submission_id) = claim_state(&s.store, grant.claim.id).await;
    assert_eq!(status, "active");
    assert_eq!(submission_id, Some(f.submission_id.to_string()));
}

// --- Review claim acquisition gates ---

#[tokio::test]
async fn review_claim_pins_pending_plan_submission() {
    let s = setup().await;
    let f = pending_plan_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;

    let grant = try_claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap();

    let (status, _, submission_id) = claim_state(&s.store, grant.claim.id).await;
    assert_eq!(status, "active");
    assert_eq!(submission_id, Some(f.submission_id.to_string()));
    let (_, phase, ..) = task_state(&s.store, f.task).await;
    assert_eq!(phase, "plan_review");
}

#[tokio::test]
async fn agent_cannot_claim_review_of_human_policy_submission() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let reviewer = agent(&s, "reviewer").await;

    let err = try_claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::ReviewerPolicy(_)),
        "expected reviewer_policy, got: {err:?}"
    );
}

#[tokio::test]
async fn human_owner_cannot_claim_review() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;

    let err = try_claim(
        &s,
        &s.owner,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::Forbidden(_)),
        "expected forbidden, got: {err:?}"
    );
}

#[tokio::test]
async fn review_claim_requires_the_pending_submission_id() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;

    let err = try_claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(Uuid::nil()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::SubmissionSuperseded(_)),
        "expected submission_superseded, got: {err:?}"
    );
}

#[tokio::test]
async fn review_claim_without_pending_submission_is_not_eligible() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;

    let err = try_claim(
        &s,
        &reviewer,
        project,
        task.id,
        ClaimPhase::Review,
        Some(Uuid::nil()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::NotEligible(_)),
        "expected not_eligible, got: {err:?}"
    );
}

#[tokio::test]
async fn concurrent_review_claims_have_one_winner() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer_a = agent(&s, "reviewer-a").await;
    let reviewer_b = agent(&s, "reviewer-b").await;
    let store_b = open_store(&s.dir, s.clock.clone()).await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, f.task).await;
    let input = || ClaimInput {
        phase: ClaimPhase::Review,
        submission_id: Some(f.submission_id),
        ttl_seconds: None,
    };
    let (first, second) = tokio::join!(
        s.store.claim_task(
            ctx(&reviewer_a, s.clock.now(), Some(revision)),
            f.project,
            f.task,
            input(),
            "claim-a",
        ),
        store_b.claim_task(
            ctx(&reviewer_b, s.clock.now(), Some(revision)),
            f.project,
            f.task,
            input(),
            "claim-b",
        ),
    );

    let outcomes = [first.is_ok(), second.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "exactly one review claim must win: {first:?} vs {second:?}"
    );
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM claims WHERE task_id = ?1 AND status = 'active'")
            .bind(f.task.to_string())
            .fetch_one(s.store.pool())
            .await
            .unwrap();
    assert_eq!(active, 1);
}

#[tokio::test]
async fn review_claim_leaves_task_unchanged() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;

    let before = task_state(&s.store, f.task).await;
    let task_events_before = event_type_count(&s.store, "task.changed").await;
    claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    let after = task_state(&s.store, f.task).await;
    assert_eq!(after, before, "review claims must not touch the task row");
    assert_eq!(
        event_type_count(&s.store, "task.changed").await,
        task_events_before,
        "review claims emit claim.changed only"
    );
    assert!(event_type_count(&s.store, "claim.changed").await >= 1);
}

// --- Reads: queue ordering, filters, cursors, membership ---

#[tokio::test]
async fn submissions_list_newest_first_with_filters_and_cursor() {
    use shepherd_core::model::{SubmissionKind, SubmissionStatus};
    use shepherd_core::queries::{ListParams, SubmissionListFilters};

    let s = setup().await;
    // Task A: one rejected and one pending work submission.
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let first_submission = f.submission_id;
    try_review(
        &s,
        &s.owner,
        f.project,
        first_submission,
        Some(1),
        ReviewDecision::Reject,
        "redo",
        None,
    )
    .await
    .unwrap();
    let (claim_id, token) = claim(
        &s,
        &f.producer,
        f.project,
        f.task,
        ClaimPhase::Execute,
        None,
    )
    .await;
    let output = create_doc(&s, f.project, f.task, DocumentKind::Finding).await;
    report_success(
        &s,
        &f.producer,
        f.project,
        claim_id,
        &token,
        vec![output.latest_revision_id.as_uuid()],
    )
    .await;
    let second_submission = pending_submission_id(&s.store, f.task).await;

    let all = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(all.items.len(), 2);
    assert_eq!(all.items[0].id, second_submission, "newest first");
    assert_eq!(all.items[1].id, first_submission);
    assert!(all.next_cursor.is_none());

    let rejected = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters {
                status: Some(SubmissionStatus::Rejected),
                ..Default::default()
            },
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.items.len(), 1);
    assert_eq!(rejected.items[0].id, first_submission);

    let work_for_task = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters {
                task_id: Some(f.task),
                kind: Some(SubmissionKind::Work),
                policy: Some(ReviewPolicy::Human),
                ..Default::default()
            },
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(work_for_task.items.len(), 2);

    // Keyset page walk: newest on page one, cursor reaches the older row.
    let page_one = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters::default(),
            &ListParams {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_one.items[0].id, second_submission);
    let cursor = page_one.next_cursor.clone().unwrap();
    let page_two = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters::default(),
            &ListParams {
                limit: Some(1),
                cursor: Some(cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_two.items[0].id, first_submission);

    // A cursor minted for a different filter fingerprint is rejected.
    let err = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters {
                status: Some(SubmissionStatus::Pending),
                ..Default::default()
            },
            &ListParams {
                limit: Some(1),
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::InvalidCursor(_)),
        "expected invalid_cursor, got: {err:?}"
    );
    let err = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters::default(),
            &ListParams {
                cursor: Some("tampered".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::InvalidCursor(_)),
        "expected invalid_cursor, got: {err:?}"
    );
}

#[tokio::test]
async fn submission_reads_enforce_project_membership() {
    use shepherd_core::queries::{ListParams, SubmissionListFilters};

    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;

    let submission = s
        .store
        .get_submission(&f.project, &f.submission_id)
        .await
        .unwrap();
    assert_eq!(submission.task_id, f.task);
    assert_eq!(submission.producer_id, f.producer.id);
    assert_eq!(submission.status.as_str(), "pending");
    assert_eq!(submission.kind.as_str(), "work");
    assert_eq!(submission.policy.as_str(), "human");
    assert_eq!(submission.document_revision_ids.len(), 1);

    let foreign = create_project(&s, "Other").await;
    let err = s
        .store
        .get_submission(&foreign.id, &f.submission_id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let err = s
        .store
        .get_submission(&f.project, &Uuid::nil())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let err = s
        .store
        .list_submissions(
            &f.project,
            &SubmissionListFilters {
                task_id: Some(TaskId::from_uuid(Uuid::nil())),
                ..Default::default()
            },
            &ListParams::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    let err = s
        .store
        .list_submission_reviews(&f.project, &Uuid::nil(), &ListParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn submission_reviews_list_returns_the_decision() {
    use shepherd_core::queries::ListParams;

    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Reject,
        "not enough proof",
        None,
    )
    .await
    .unwrap();

    let page = s
        .store
        .list_submission_reviews(&f.project, &f.submission_id, &ListParams::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let review = &page.items[0];
    assert_eq!(review.submission_id, f.submission_id);
    assert_eq!(review.actor_id, s.owner.id);
    assert_eq!(review.decision.as_str(), "reject");
    assert_eq!(review.reason, "not enough proof");
    assert!(page.next_cursor.is_none());
}

// --- Revocation closes the reviewer's claim (plan/03, plan/12) ---

#[tokio::test]
async fn revoking_reviewer_closes_claim_and_frees_the_review() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    let revision_before = task_revision(&s.store, f.task).await;
    let tasks_before = event_type_count(&s.store, "task.changed").await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .revoke_agent(
            ctx(&s.owner, s.clock.now(), None),
            reviewer.id,
            "compromised credential".to_string(),
            "revoke-hash",
        )
        .await
        .unwrap();

    let (claim_status, close_reason, _) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "revoked");
    assert_eq!(close_reason, "compromised credential");
    // The revoked claim changed the task's representation: bump plus event,
    // matching the block/cancel revocation paths.
    assert_eq!(task_revision(&s.store, f.task).await, revision_before + 1);
    assert_eq!(
        event_type_count(&s.store, "task.changed").await,
        tasks_before + 1
    );

    // The revoked principal cannot decide.
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((claim_id, &token)),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::Forbidden(_)),
        "expected forbidden for a revoked actor, got: {err:?}"
    );

    // The submission stays pending and a distinct agent takes over
    // immediately — no TTL wait behind the revoked claim.
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "pending");
    let replacement = agent(&s, "replacement").await;
    let (new_claim, new_token) = claim(
        &s,
        &replacement,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;
    try_review(
        &s,
        &replacement,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((new_claim, &new_token)),
    )
    .await
    .unwrap();
    let (status, phase, ..) = task_state(&s.store, f.task).await;
    assert_eq!(status, "done");
    assert_eq!(phase, "complete");
}

// --- Acceptance: none policy creates accepted submission without fake human Review ---

#[tokio::test]
async fn none_policy_accepts_submission_without_fake_review() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None, ReviewPolicy::None).await;
    let producer = agent(&s, "producer").await;
    let (claim_id, token) = claim(&s, &producer, project, task.id, ClaimPhase::Execute, None).await;
    let output = create_doc(&s, project, task.id, DocumentKind::Finding).await;
    report_success(
        &s,
        &producer,
        project,
        claim_id,
        &token,
        vec![output.latest_revision_id.as_uuid()],
    )
    .await;

    let (sub_status,): (String,) =
        sqlx::query_as("SELECT status FROM submissions WHERE task_id = ?1")
            .bind(task.id.to_string())
            .fetch_one(s.store.pool())
            .await
            .unwrap();
    assert_eq!(sub_status, "accepted");
    let (status, phase, ..) = task_state(&s.store, task.id).await;
    assert_eq!(status, "done");
    assert_eq!(phase, "complete");
    assert_eq!(reviews_count(&s.store).await, 0, "no fabricated review");
    assert_eq!(event_type_count(&s.store, "review.recorded").await, 0);
}

// --- Expiry: late review fails, the queue is not deadlocked ---

#[tokio::test]
async fn expired_review_claim_fails_late_decision_and_frees_the_queue() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    // Past the 300s default TTL: the lease no longer authorizes a decision.
    s.clock.advance(TimeDelta::seconds(1_000));
    let err = try_review(
        &s,
        &reviewer,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((claim_id, &token)),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::LeaseInvalid),
        "expected lease_invalid after expiry, got: {err:?}"
    );
    // The failed decision rolled back whole, reconcile included: the raw row
    // still says active until the next successful command sweeps it.
    let (claim_status, ..) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "active");

    // A distinct agent claims the still-pending submission and decides.
    let replacement = agent(&s, "replacement").await;
    let (new_claim, new_token) = claim(
        &s,
        &replacement,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;
    // Acquiring the replacement claim reconciled the stale one to expired.
    let (claim_status, ..) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "expired");
    try_review(
        &s,
        &replacement,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((new_claim, &new_token)),
    )
    .await
    .unwrap();
    let (status, ..) = task_state(&s.store, f.task).await;
    assert_eq!(status, "done");
}

// --- Blocking: claim revoked, submission stays pending ---

#[tokio::test]
async fn blocking_task_revokes_review_claim_but_submission_stays_pending() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, _token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;

    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, f.task).await;
    s.store
        .block_task(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            f.project,
            f.task,
            "hold the line".to_string(),
        )
        .await
        .unwrap();

    let (claim_status, ..) = claim_state(&s.store, claim_id).await;
    assert_eq!(claim_status, "revoked");
    let (sub_status, ..) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "pending", "block keeps the submission pending");

    // Blocked tasks accept no new review claim.
    let other = agent(&s, "other").await;
    let err = try_claim(
        &s,
        &other,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::NotEligible(_)),
        "expected not_eligible while blocked, got: {err:?}"
    );

    // Unblock: the pending review resumes.
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, f.task).await;
    s.store
        .unblock_task(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            f.project,
            f.task,
        )
        .await
        .unwrap();
    let (new_claim, new_token) = claim(
        &s,
        &other,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(f.submission_id),
    )
    .await;
    try_review(
        &s,
        &other,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        Some((new_claim, &new_token)),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn blocked_task_rejects_human_decision_as_not_eligible() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, f.task).await;
    s.store
        .block_task(
            ctx(&s.owner, s.clock.now(), Some(revision)),
            f.project,
            f.task,
            "frozen".to_string(),
        )
        .await
        .unwrap();

    let err = try_review(
        &s,
        &s.owner,
        f.project,
        f.submission_id,
        Some(1),
        ReviewDecision::Approve,
        "",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::NotEligible(_)),
        "expected not_eligible while blocked, got: {err:?}"
    );
}

// --- Superseded: an old submission id never matches a new pending one ---

#[tokio::test]
async fn review_claim_with_superseded_submission_id_fails() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Agent).await;
    let old_submission = f.submission_id;
    let reviewer = agent(&s, "reviewer").await;
    let (claim_id, token) = claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(old_submission),
    )
    .await;
    try_review(
        &s,
        &reviewer,
        f.project,
        old_submission,
        Some(1),
        ReviewDecision::Reject,
        "insufficient",
        Some((claim_id, &token)),
    )
    .await
    .unwrap();

    // Producer re-executes; a new pending submission replaces the rejected one.
    let (claim_id, token) = claim(
        &s,
        &f.producer,
        f.project,
        f.task,
        ClaimPhase::Execute,
        None,
    )
    .await;
    let output = create_doc(&s, f.project, f.task, DocumentKind::Finding).await;
    report_success(
        &s,
        &f.producer,
        f.project,
        claim_id,
        &token,
        vec![output.latest_revision_id.as_uuid()],
    )
    .await;

    let err = try_claim(
        &s,
        &reviewer,
        f.project,
        f.task,
        ClaimPhase::Review,
        Some(old_submission),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DomainError::SubmissionSuperseded(_)),
        "expected submission_superseded for the old id, got: {err:?}"
    );
}

// --- Withdrawal replays idempotently ---

#[tokio::test]
async fn withdrawal_replays_idempotently() {
    let s = setup().await;
    let f = pending_work_submission(&s, ReviewPolicy::Human).await;
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));

    s.clock.advance(TimeDelta::milliseconds(2));
    let first = s
        .store
        .withdraw_submission(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), key),
            f.project,
            f.submission_id,
            "rethinking".to_string(),
            "withdraw-replay",
        )
        .await
        .unwrap();
    assert!(!first.is_replay());

    s.clock.advance(TimeDelta::milliseconds(2));
    let second = s
        .store
        .withdraw_submission(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), key),
            f.project,
            f.submission_id,
            "rethinking".to_string(),
            "withdraw-replay",
        )
        .await
        .unwrap();
    assert!(second.is_replay());
    let (sub_status, sub_revision, _) = submission_state(&s.store, f.submission_id).await;
    assert_eq!(sub_status, "withdrawn");
    assert_eq!(sub_revision, 2, "replay does not withdraw twice");
}
