use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimGrant, ClaimInput, ClaimPhase, Clock, CommandId, Document,
    DocumentCreate, DocumentKind, DocumentOwner, Epic, EpicCreate, EpicId, GoalCreate, GoalId,
    Project, ProjectCreate, ProjectId, ReportInput, ReviewPolicy, SelectPlanInput, SessionOutcome,
    Task, TaskCreate, TaskId, TestClock,
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
    s.store
        .create_document(
            ctx(&s.owner, s.clock.now(), None),
            project,
            DocumentCreate {
                owner: DocumentOwner::Task(task),
                kind,
                title: "Doc".to_string(),
                body: "# body".to_string(),
                links: Vec::new(),
            },
            "doc-hash",
        )
        .await
        .unwrap()
        .into_inner()
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
