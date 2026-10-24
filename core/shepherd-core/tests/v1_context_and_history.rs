// Step 013 acceptance: context assembly and durable history (CONTENT-01, DATA-01).
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimGrant, ClaimInput, ClaimPhase, Clock, CommandId,
    DependencyCreate, Document, DocumentCreate, DocumentKind, DocumentOwner, DocumentRevision,
    Epic, EpicCreate, EpicId, GoalCreate, GoalId, Project, ProjectCreate, ProjectId, ReportInput,
    ReviewDecision, ReviewInput, ReviewPolicy, RevisionCreate, SelectPlanInput, SessionOutcome,
    Task, TaskCreate, TaskId, TestClock, TextPatch,
};
use shepherd_core::queries::{ListParams, ReplayOutcome, ResyncReason};
use shepherd_core::storage::rows::insert_actor;
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

fn ctx_with_key(actor: &Actor, now: DateTime<Utc>, key: Uuid) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: key,
        expected_revision: None,
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
    s.clock.advance(TimeDelta::milliseconds(2));
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
    s.clock.advance(TimeDelta::milliseconds(2));
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
) -> (ProjectId, Epic, Task) {
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
    (project.id, epic, task)
}

async fn create_document(
    s: &Setup,
    project: ProjectId,
    owner: DocumentOwner,
    kind: DocumentKind,
    body: &str,
) -> Document {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_document(
            ctx(&s.owner, s.clock.now(), None),
            project,
            DocumentCreate {
                owner,
                kind,
                title: format!("{} doc", kind.as_str()),
                body: body.to_string(),
                links: Vec::new(),
            },
            "doc-hash",
        )
        .await
        .unwrap()
        .into_inner()
}

async fn add_revision(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    document: &Document,
    expected_revision: i64,
    body: &str,
) -> DocumentRevision {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_document_revision(
            ctx(who, s.clock.now(), Some(expected_revision)),
            project,
            document.id,
            RevisionCreate {
                body: body.to_string(),
                links: Vec::new(),
            },
            "revision-hash",
        )
        .await
        .unwrap()
        .into_inner()
}

async fn claim(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    task: TaskId,
    phase: ClaimPhase,
) -> ClaimGrant {
    s.clock.advance(TimeDelta::milliseconds(2));
    let revision = task_revision(&s.store, task).await;
    s.store
        .claim_task(
            ctx(who, s.clock.now(), Some(revision)),
            project,
            task,
            ClaimInput {
                phase,
                submission_id: None,
                ttl_seconds: None,
            },
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner()
}

#[allow(clippy::too_many_arguments)]
async fn report(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    claim_id: Uuid,
    token: &str,
    outcome: SessionOutcome,
    summary: &str,
    revisions: Vec<Uuid>,
) -> Result<(), DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(who, s.clock.now(), None),
            project,
            claim_id,
            token,
            ReportInput {
                outcome,
                summary: summary.into(),
                failure_reason: match outcome {
                    SessionOutcome::Succeeded => String::new(),
                    SessionOutcome::Failed => "step failed".into(),
                },
                document_revision_ids: revisions,
                links: serde_json::json!([]),
            },
            "report-hash",
        )
        .await
        .map(|_| ())
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

async fn review(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    submission: Uuid,
    decision: ReviewDecision,
    reason: &str,
    claim: Option<(Uuid, &str)>,
) {
    s.clock.advance(TimeDelta::milliseconds(2));
    let (claim_id, token) = match claim {
        Some((id, token)) => (Some(id), Some(token)),
        None => (None, None),
    };
    let revision = submission_revision(&s.store, submission).await;
    s.store
        .review_submission(
            ctx(who, s.clock.now(), Some(revision)),
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

async fn submission_revision(store: &Store, submission: Uuid) -> i64 {
    sqlx::query_scalar("SELECT revision FROM submissions WHERE id = ?1")
        .bind(submission.to_string())
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

async fn event_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn last_event_id(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// (id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, action, reason)
type EventRow = (
    i64,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
);

async fn events_after(store: &Store, marker: i64) -> Vec<EventRow> {
    sqlx::query_as(
        "SELECT id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, \
         action, reason FROM events WHERE id > ?1 ORDER BY id ASC",
    )
    .bind(marker)
    .fetch_all(store.pool())
    .await
    .unwrap()
}

fn kind_rank(event_type: &str) -> u8 {
    match event_type {
        "project.changed" => 0,
        "goal.changed" => 1,
        "epic.changed" => 2,
        "task.changed" => 3,
        "task_type.changed" => 4,
        "dependency.changed" => 5,
        "claim.changed" => 6,
        "submission.changed" => 7,
        "session.recorded" => 8,
        "document.changed" => 9,
        "review.recorded" => 10,
        other => panic!("unknown event type {other}"),
    }
}

/// One coherent event group (plan/08): a single command_id whose rows share
/// actor/occurred_at/action/reason, are ordered by (kind rank, resource UUID),
/// and bump each resource at most once.
fn assert_group(rows: &[EventRow], required: &[&str], exact: bool) {
    assert!(!rows.is_empty(), "expected an event group");
    let (_, _, _, _, command_id, actor_id, occurred_at, action, reason) = rows[0].clone();
    for row in rows {
        assert_eq!(row.4, command_id, "one command_id per group");
        assert_eq!(row.5, actor_id, "one actor per group");
        assert_eq!(row.6, occurred_at, "one occurred_at per group");
        assert_eq!(row.7, action, "one action per group");
        assert_eq!(row.8, reason, "one reason per group");
    }
    let keys: Vec<(u8, String)> = rows
        .iter()
        .map(|row| (kind_rank(&row.1), row.2.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "rows ordered by resource kind then UUID");
    let mut pairs: Vec<(String, String)> = rows
        .iter()
        .map(|row| (row.1.clone(), row.2.clone()))
        .collect();
    pairs.sort();
    pairs.dedup();
    assert_eq!(pairs.len(), rows.len(), "one bump per resource per command");
    let mut types: Vec<&str> = rows.iter().map(|row| row.1.as_str()).collect();
    types.sort();
    let mut expected: Vec<&str> = required.to_vec();
    expected.sort();
    if exact {
        assert_eq!(types, expected, "exact event-type set");
    } else {
        for want in expected {
            assert!(types.contains(&want), "missing {want} in {types:?}");
        }
    }
}

fn history_params(limit: Option<u32>, cursor: Option<String>) -> ListParams {
    ListParams {
        limit,
        cursor,
        include_archived: false,
    }
}

// --- Workflow fixtures ---

struct AcceptedPlan {
    project: ProjectId,
    task: Task,
    plan_document: Document,
    first_revision: Uuid,
}

/// Plan flow to acceptance under the human gate, with a second plan revision
/// appended so the document's latest differs from the pinned selection.
async fn accepted_plan_with_newer_revision(s: &Setup) -> AcceptedPlan {
    let (project, _, task) = scaffold(s, true, ReviewPolicy::Human, ReviewPolicy::None).await;
    let producer = agent(s, "producer").await;
    let grant = claim(s, &producer, project, task.id, ClaimPhase::Plan).await;
    let document = create_document(
        s,
        project,
        DocumentOwner::Task(task.id),
        DocumentKind::Plan,
        "plan v1",
    )
    .await;
    let first_revision = document.latest_revision_id.as_uuid();
    add_revision(s, &producer, project, &document, 1, "plan v2 (newer)").await;
    select_plan(s, project, task.id, first_revision).await;
    report(
        s,
        &producer,
        project,
        grant.claim.id,
        &grant.lease_token,
        SessionOutcome::Succeeded,
        "planned",
        vec![first_revision],
    )
    .await
    .unwrap();
    let submission = pending_submission_id(&s.store, task.id).await;
    review(
        s,
        &s.owner,
        project,
        submission,
        ReviewDecision::Approve,
        "plan looks good",
        None,
    )
    .await;
    AcceptedPlan {
        project,
        task,
        plan_document: document,
        first_revision,
    }
}

/// N failed execute attempts, each recording one immutable session.
async fn failed_attempts(
    s: &Setup,
    who: &Actor,
    project: ProjectId,
    task: TaskId,
    count: usize,
    summary: &str,
) {
    for _ in 0..count {
        let grant = claim(s, who, project, task, ClaimPhase::Execute).await;
        report(
            s,
            who,
            project,
            grant.claim.id,
            &grant.lease_token,
            SessionOutcome::Failed,
            summary,
            Vec::new(),
        )
        .await
        .unwrap();
    }
}

fn context_paths(project: ProjectId, epic: EpicId, task: TaskId) -> [String; 6] {
    let prefix = format!("/api/v1/projects/{project}");
    [
        format!("{prefix}/tasks/{task}/sessions"),
        format!("{prefix}/submissions?task_id={task}"),
        format!("{prefix}/documents?owner_kind=project&owner_id={project}"),
        format!("{prefix}/documents?owner_kind=epic&owner_id={epic}"),
        format!("{prefix}/claims?active=true"),
        format!("{prefix}/dependencies"),
    ]
}

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
        vec![epic_doc.id.as_uuid(), project_doc.id.as_uuid()],
        "newest first, project/epic owners only"
    );
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

/// Scaffold plus enough goal updates to cross the default page size.
async fn seeded_history(s: &Setup) -> (ProjectId, GoalId) {
    let project = create_project(s, "P").await;
    let goal = create_goal(s, project.id, "G").await;
    for round in 0..55 {
        s.clock.advance(TimeDelta::milliseconds(2));
        s.store
            .update_goal(
                ctx(&s.owner, s.clock.now(), Some(round + 1)),
                project.id,
                goal,
                TextPatch {
                    title: Some(format!("G{round}")),
                    description: None,
                },
            )
            .await
            .unwrap();
    }
    (project.id, goal)
}

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

/// Raw synthetic event rows for backlog bounds; commands cannot mint thousands
/// of events deterministically fast.
async fn seed_events(s: &Setup, project: ProjectId, count: usize) {
    let actor = s.owner.id.to_string();
    let project_text = project.to_string();
    let occurred = shepherd_core::storage::rows::format_ts(&s.clock.now());
    s.store
        .command_transaction(move |tx| {
            Box::pin(async move {
                for index in 0..count {
                    sqlx::query(
                        "INSERT INTO events (action, reason, project_id, actor_id, command_id, \
                         type, resource_id, resource_revision, occurred_at, affected_ids) \
                         VALUES ('seed', '', ?1, ?2, ?3, 'task.changed', ?4, ?5, ?6, '[]')",
                    )
                    .bind(&project_text)
                    .bind(&actor)
                    .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
                    .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
                    .bind(index as i64 + 1)
                    .bind(&occurred)
                    .execute(&mut **tx)
                    .await?;
                }
                Ok(())
            })
        })
        .await
        .unwrap();
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
