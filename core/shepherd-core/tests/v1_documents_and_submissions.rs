use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimInput, ClaimPhase, Clock, CommandId, Document, DocumentCreate,
    DocumentKind, DocumentOwner, Epic, EpicCreate, EpicId, GoalCreate, GoalId, Project,
    ProjectCreate, ProjectId, ReportInput, ReviewPolicy, RevisionCreate, SelectPlanInput,
    SessionOutcome, Task, TaskCreate, TaskId, TaskPhase, TestClock,
};
use shepherd_core::queries::ListParams;
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
        request_hash: Some("test-request".into()),
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
        request_hash: Some("test-request".into()),
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
        ReviewPolicy::None,
    )
    .await;
    (project.id, epic.id, task)
}

fn doc_input(owner: DocumentOwner, kind: DocumentKind, title: &str) -> DocumentCreate {
    DocumentCreate {
        owner,
        kind,
        title: title.to_string(),
        body: "# body".to_string(),
        links: Vec::new(),
    }
}

async fn create_doc(s: &Setup, project: ProjectId, input: DocumentCreate) -> Document {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_document(
            ctx(&s.owner, s.clock.now(), None),
            project,
            input,
            "doc-hash",
        )
        .await
        .unwrap()
        .into_inner()
}

async fn try_append(
    s: &Setup,
    project: ProjectId,
    document: &Document,
    expected_revision: Option<i64>,
    body: &str,
) -> Result<shepherd_core::model::DocumentRevision, DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .create_document_revision(
            ctx(&s.owner, s.clock.now(), expected_revision),
            project,
            document.id,
            RevisionCreate {
                body: body.to_string(),
                links: Vec::new(),
            },
            "rev-hash",
        )
        .await
        .map(shepherd_core::commands::Replay::into_inner)
}

async fn try_select(
    s: &Setup,
    project: ProjectId,
    task: TaskId,
    revision_id: Uuid,
) -> Result<Task, DomainError> {
    s.clock.advance(TimeDelta::milliseconds(2));
    let (_, _, _, task_revision) = task_plan_state(&s.store, task).await;
    s.store
        .select_task_plan(
            ctx(&s.owner, s.clock.now(), Some(task_revision)),
            project,
            task,
            SelectPlanInput {
                document_revision_id: revision_id,
            },
        )
        .await
        .map(|result| result.value)
}

async fn claim(s: &Setup, project: ProjectId, task: TaskId, phase: ClaimPhase) -> (Uuid, String) {
    s.clock.advance(TimeDelta::milliseconds(2));
    let (_, _, _, task_revision) = task_plan_state(&s.store, task).await;
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_revision)),
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
        .into_inner();
    (grant.claim.id, grant.lease_token)
}

async fn report_plan_success(
    s: &Setup,
    project: ProjectId,
    claim_id: Uuid,
    token: &str,
    revisions: Vec<Uuid>,
) {
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            claim_id,
            token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "planned".into(),
                failure_reason: String::new(),
                document_revision_ids: revisions,
                links: serde_json::json!([]),
            },
            "plan-report-hash",
        )
        .await
        .unwrap();
}

struct PlanFlow {
    project: ProjectId,
    task: TaskId,
    document: Document,
    v1: Uuid,
    v2: Uuid,
}

/// Public plan flow: claim plan, create the plan document, append a second
/// revision, select it, report plan success with both revisions.
async fn run_plan_flow(s: &Setup, plan_review: ReviewPolicy) -> PlanFlow {
    let (project, _, task) = scaffold(s, true, plan_review).await;
    let (claim_id, token) = claim(s, project, task.id, ClaimPhase::Plan).await;
    let document = create_doc(
        s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Plan, "Plan"),
    )
    .await;
    let v1 = document.latest_revision_id.as_uuid();
    let appended = try_append(s, project, &document, Some(1), "v2")
        .await
        .unwrap();
    let v2 = appended.id.as_uuid();
    try_select(s, project, task.id, v2).await.unwrap();
    report_plan_success(s, project, claim_id, &token, vec![v1, v2]).await;
    let document = s.store.get_document(&project, &document.id).await.unwrap();
    PlanFlow {
        project,
        task: task.id,
        document,
        v1,
        v2,
    }
}

// --- Persisted-state probes ---

async fn document_state(store: &Store, document: &Document) -> (i64, String) {
    sqlx::query_as("SELECT revision, latest_revision_id FROM documents WHERE id = ?1")
        .bind(document.id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn revision_count(store: &Store, document: &Document) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM document_revisions WHERE document_id = ?1")
        .bind(document.id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn task_plan_state(
    store: &Store,
    task: TaskId,
) -> (Option<String>, Option<String>, String, i64) {
    sqlx::query_as(
        "SELECT selected_plan_revision_id, accepted_plan_submission_id, phase, revision \
         FROM tasks WHERE id = ?1",
    )
    .bind(task.to_string())
    .fetch_one(store.pool())
    .await
    .unwrap()
}

async fn event_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM events")
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

// --- Acceptance: new document has revision 1 ---

#[tokio::test]
async fn new_document_has_revision_one() {
    let s = setup().await;
    let (project, epic, task) = scaffold(&s, false, ReviewPolicy::None).await;
    for (owner, kind) in [
        (DocumentOwner::Project(project), DocumentKind::Brief),
        (DocumentOwner::Epic(epic), DocumentKind::Note),
        (DocumentOwner::Task(task.id), DocumentKind::Plan),
    ] {
        let document = create_doc(&s, project, doc_input(owner, kind, "Doc")).await;
        assert_eq!(document.revision.value(), 1);
        assert_eq!(document.owner_id, owner.id());
        let revision = s
            .store
            .get_document_revision(&project, &document.latest_revision_id)
            .await
            .unwrap();
        assert_eq!(revision.number, 1);
        assert_eq!(revision.body, "# body");
        assert_eq!(revision.document_id, document.id);
        assert_eq!(revision_count(&s.store, &document).await, 1);
    }
    let document_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE type = 'document.changed' AND action = 'createDocument'",
    )
    .fetch_one(s.store.pool())
    .await
    .unwrap();
    assert_eq!(document_events, 3);
}

// --- Acceptance: stale append returns 412 ---

#[tokio::test]
async fn stale_append_returns_revision_conflict() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
    )
    .await;
    let second = try_append(&s, project, &document, Some(1), "v2")
        .await
        .unwrap();
    assert_eq!(second.number, 2);

    let events_before = event_count(&s.store).await;
    let err = try_append(&s, project, &document, Some(1), "stale")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::RevisionConflict {
            expected: 1,
            actual: 2
        }
    ));
    let (revision, latest) = document_state(&s.store, &document).await;
    assert_eq!(revision, 2);
    assert_eq!(latest, second.id.to_string());
    assert_eq!(revision_count(&s.store, &document).await, 2);
    assert_eq!(event_count(&s.store).await, events_before);
}

#[tokio::test]
async fn append_without_if_match_requires_precondition() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
    )
    .await;
    let events_before = event_count(&s.store).await;
    let err = try_append(&s, project, &document, None, "v2")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PreconditionRequired));
    assert_eq!(revision_count(&s.store, &document).await, 1);
    assert_eq!(event_count(&s.store).await, events_before);
}

// --- Acceptance: accepted plan revision remains readable ---

#[tokio::test]
async fn accepted_plan_revision_remains_readable() {
    let s = setup().await;
    let flow = run_plan_flow(&s, ReviewPolicy::None).await;
    let (selected, accepted, phase, _) = task_plan_state(&s.store, flow.task).await;
    assert_eq!(selected.as_deref(), Some(flow.v2.to_string().as_str()));
    assert!(accepted.is_some());
    assert_eq!(phase, TaskPhase::Execution.as_str());

    // Appending outside execute/review claims is allowed and never selects.
    let v3 = try_append(&s, flow.project, &flow.document, Some(2), "v3")
        .await
        .unwrap();
    assert_eq!(v3.number, 3);
    let (selected_after, accepted_after, _, _) = task_plan_state(&s.store, flow.task).await;
    assert_eq!(selected_after, selected);
    assert_eq!(accepted_after, accepted);

    let accepted_revision = s
        .store
        .get_document_revision(
            &flow.project,
            &shepherd_core::model::DocumentRevisionId::from_uuid(flow.v2),
        )
        .await
        .unwrap();
    assert_eq!(accepted_revision.number, 2);
    assert_eq!(accepted_revision.body, "v2");

    let page = s
        .store
        .list_document_revisions(&flow.project, &flow.document.id, &ListParams::default())
        .await
        .unwrap();
    let numbers: Vec<i64> = page.items.iter().map(|r| r.number).collect();
    assert_eq!(numbers, vec![3, 2, 1], "revisions list newest first");
}

// --- Acceptance: changing selected plan clears approval ---

#[tokio::test]
async fn changing_selected_plan_clears_approval() {
    let s = setup().await;
    let flow = run_plan_flow(&s, ReviewPolicy::None).await;
    let (_, _, _, revision_before) = task_plan_state(&s.store, flow.task).await;

    let updated = try_select(&s, flow.project, flow.task, flow.v1)
        .await
        .unwrap();
    assert_eq!(updated.phase, TaskPhase::Planning);
    let (selected, accepted, phase, revision) = task_plan_state(&s.store, flow.task).await;
    assert_eq!(selected.as_deref(), Some(flow.v1.to_string().as_str()));
    assert_eq!(accepted, None, "changing the selected plan clears approval");
    assert_eq!(phase, TaskPhase::Planning.as_str());
    assert_eq!(revision, revision_before + 1);
    let select_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE action = 'selectTaskPlan' AND type = 'task.changed'",
    )
    .fetch_one(s.store.pool())
    .await
    .unwrap();
    assert_eq!(select_events, 2, "flow selection plus this change");
}

#[tokio::test]
async fn reselecting_the_same_plan_is_a_noop() {
    let s = setup().await;
    let flow = run_plan_flow(&s, ReviewPolicy::None).await;
    let state_before = task_plan_state(&s.store, flow.task).await;
    let events_before = event_count(&s.store).await;

    let returned = try_select(&s, flow.project, flow.task, flow.v2)
        .await
        .unwrap();
    assert_eq!(returned.phase, TaskPhase::Execution);
    assert_eq!(returned.revision.value(), state_before.3);
    assert_eq!(
        task_plan_state(&s.store, flow.task).await,
        state_before,
        "re-selecting the current revision changes nothing"
    );
    assert_eq!(event_count(&s.store).await, events_before);
}

// --- Acceptance: claimed execution cannot select another plan ---

#[tokio::test]
async fn claimed_execution_cannot_select_another_plan() {
    let s = setup().await;
    let flow = run_plan_flow(&s, ReviewPolicy::None).await;
    let (claim_id, _token) = claim(&s, flow.project, flow.task, ClaimPhase::Execute).await;
    let state_before = task_plan_state(&s.store, flow.task).await;
    let events_before = event_count(&s.store).await;

    let err = try_select(&s, flow.project, flow.task, flow.v1)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));
    assert_eq!(task_plan_state(&s.store, flow.task).await, state_before);
    assert_eq!(event_count(&s.store).await, events_before);
    assert_eq!(claim_status(&s.store, claim_id).await, "active");
}

#[tokio::test]
async fn pending_submission_blocks_selection_and_selected_plan_appends() {
    let s = setup().await;
    // Human plan review leaves a pending submission after the plan report.
    let flow = run_plan_flow(&s, ReviewPolicy::Human).await;
    let (_, accepted, phase, _) = task_plan_state(&s.store, flow.task).await;
    assert_eq!(accepted, None);
    assert_eq!(phase, TaskPhase::PlanReview.as_str());

    let err = try_select(&s, flow.project, flow.task, flow.v1)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));

    let err = try_append(&s, flow.project, &flow.document, Some(2), "v3")
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::ActiveWork(_)),
        "selected plan must not gain revisions while a submission is pending"
    );
    assert_eq!(revision_count(&s.store, &flow.document).await, 2);
}

// --- Acceptance: documents retrievable in later sessions ---

#[tokio::test]
async fn project_and_epic_documents_are_retrievable_in_later_sessions() {
    let s = setup().await;
    let (project, epic, _) = scaffold(&s, false, ReviewPolicy::None).await;
    let project_doc = create_doc(
        &s,
        project,
        doc_input(
            DocumentOwner::Project(project),
            DocumentKind::Brief,
            "Brief",
        ),
    )
    .await;
    let epic_doc = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Epic(epic), DocumentKind::Handoff, "Handoff"),
    )
    .await;

    // A later session: a fresh pool over the same database file.
    let later = open_store(&s.dir, s.clock.clone()).await;
    let reloaded = later.get_document(&project, &project_doc.id).await.unwrap();
    assert_eq!(reloaded, project_doc);
    let revision = later
        .get_document_revision(&project, &epic_doc.latest_revision_id)
        .await
        .unwrap();
    assert_eq!(revision.body, "# body");
    let page = later
        .list_documents(
            &project,
            &shepherd_core::commands::DocumentListFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
}

// --- Acceptance: unsafe links and oversized content fail ---

#[tokio::test]
async fn unsafe_links_and_oversized_content_fail() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
    )
    .await;
    let events_before = event_count(&s.store).await;
    let docs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
        .fetch_one(s.store.pool())
        .await
        .unwrap();

    let unsafe_links = vec![
        vec!["javascript:alert(1)".to_string()],
        vec!["data:text/html;base64,AAAA".to_string()],
        vec!["file:///etc/passwd".to_string()],
        vec!["http://user:secret@example.com".to_string()],
        vec!["ftp://example.com".to_string()],
    ];
    for links in unsafe_links {
        let err = s
            .store
            .create_document(
                ctx(&s.owner, s.clock.now(), None),
                project,
                DocumentCreate {
                    links: links.clone(),
                    ..doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Bad")
                },
                "bad-doc-hash",
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Validation { field: "links", .. }),
            "{links:?} must be rejected"
        );
    }

    let oversize_cases = [
        DocumentCreate {
            body: "x".repeat(65_537),
            ..doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Bad")
        },
        DocumentCreate {
            title: "x".repeat(201),
            ..doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Bad")
        },
        DocumentCreate {
            links: vec!["https://example.com".to_string(); 51],
            ..doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Bad")
        },
    ];
    for input in oversize_cases {
        let err = s
            .store
            .create_document(
                ctx(&s.owner, s.clock.now(), None),
                project,
                input,
                "bad-hash",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { .. }));
    }

    // Appends apply identical validation.
    let err = s
        .store
        .create_document_revision(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project,
            document.id,
            RevisionCreate {
                body: "x".repeat(65_537),
                links: Vec::new(),
            },
            "bad-rev-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation { field: "body", .. }));

    let docs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    assert_eq!(docs_after, docs_before);
    assert_eq!(revision_count(&s.store, &document).await, 1);
    assert_eq!(event_count(&s.store).await, events_before);
}

// --- Concurrency over independent pools ---

#[tokio::test]
async fn concurrent_appends_have_one_winner() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
    )
    .await;

    let store_b = open_store(&s.dir, s.clock.clone()).await;
    s.clock.advance(TimeDelta::milliseconds(2));
    // Both appends start on the same revision with independent pools; only
    // one can win the command_lock write reservation.
    let now = s.clock.now();
    let (a, b) = tokio::join!(
        s.store.create_document_revision(
            ctx(&s.owner, now, Some(1)),
            project,
            document.id,
            RevisionCreate {
                body: "pool a".into(),
                links: Vec::new(),
            },
            "rev-a",
        ),
        store_b.create_document_revision(
            ctx(&s.owner, now, Some(1)),
            project,
            document.id,
            RevisionCreate {
                body: "pool b".into(),
                links: Vec::new(),
            },
            "rev-b",
        ),
    );

    let (winner, loser) = match (a, b) {
        (Ok(winner), Err(loser)) => (winner, loser),
        (Err(loser), Ok(winner)) => (winner, loser),
        (Ok(_), Ok(_)) => panic!("both concurrent appends succeeded"),
        (Err(_), Err(_)) => panic!("both concurrent appends failed"),
    };
    let winner = winner.into_inner();
    assert_eq!(winner.number, 2);
    // The loser waited for the winner's commit, re-read the bumped revision
    // and failed its stale If-Match.
    assert!(matches!(
        loser,
        DomainError::RevisionConflict {
            expected: 1,
            actual: 2
        }
    ));
    // Either pool serves the winning append, body and all.
    let body: String = sqlx::query_scalar("SELECT body FROM document_revisions WHERE number = 2")
        .fetch_one(store_b.pool())
        .await
        .unwrap();
    assert_eq!(body, winner.body);
    assert_eq!(revision_count(&store_b, &document).await, 2);
}

// --- Idempotency ---

#[tokio::test]
async fn document_commands_replay_idempotently() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;

    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    s.clock.advance(TimeDelta::milliseconds(2));
    let first = s
        .store
        .create_document(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
            "same-hash",
        )
        .await
        .unwrap();
    assert!(!first.is_replay());
    let first = first.into_inner();

    let replayed = s
        .store
        .create_document(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
            "same-hash",
        )
        .await
        .unwrap();
    assert!(replayed.is_replay());
    assert_eq!(replayed.into_inner(), first);
    assert_eq!(revision_count(&s.store, &first).await, 1);

    let err = s
        .store
        .create_document(
            ctx_with_key(&s.owner, s.clock.now(), None, key),
            project,
            doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Other"),
            "different-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::IdempotencyConflict));

    // Revision appends replay the same way.
    let rev_key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    s.clock.advance(TimeDelta::milliseconds(2));
    let appended = s
        .store
        .create_document_revision(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), rev_key),
            project,
            first.id,
            RevisionCreate {
                body: "v2".into(),
                links: Vec::new(),
            },
            "same-rev-hash",
        )
        .await
        .unwrap()
        .into_inner();
    let replayed = s
        .store
        .create_document_revision(
            ctx_with_key(&s.owner, s.clock.now(), Some(1), rev_key),
            project,
            first.id,
            RevisionCreate {
                body: "v2".into(),
                links: Vec::new(),
            },
            "same-rev-hash",
        )
        .await
        .unwrap();
    assert!(replayed.is_replay());
    assert_eq!(replayed.into_inner(), appended);
    assert_eq!(revision_count(&s.store, &first).await, 2);
}

// --- End-to-end plan claim flow ---

#[tokio::test]
async fn plan_claim_flow_selects_own_plan_and_reports_success() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::None).await;
    let (claim_id, token) = claim(&s, project, task.id, ClaimPhase::Plan).await;

    // The claimant creates, appends and selects its plan during the claim.
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Plan, "Plan"),
    )
    .await;
    let v1 = document.latest_revision_id.as_uuid();
    let v2 = try_append(&s, project, &document, Some(1), "v2")
        .await
        .unwrap()
        .id
        .as_uuid();
    try_select(&s, project, task.id, v2).await.unwrap();
    assert_eq!(
        claim_status(&s.store, claim_id).await,
        "active",
        "append and selection preserve the plan claim"
    );

    report_plan_success(&s, project, claim_id, &token, vec![v1, v2]).await;
    let (selected, accepted, phase, _) = task_plan_state(&s.store, task.id).await;
    assert_eq!(selected.as_deref(), Some(v2.to_string().as_str()));
    assert!(accepted.is_some(), "policy none accepts the plan inline");
    assert_eq!(phase, TaskPhase::Execution.as_str());
    assert_eq!(claim_status(&s.store, claim_id).await, "reported");
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE task_id = ?1")
        .bind(task.id.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
    assert_eq!(sessions, 1);
}

#[tokio::test]
async fn unselected_documents_stay_appendable_during_active_work() {
    let s = setup().await;
    let flow = run_plan_flow(&s, ReviewPolicy::None).await;
    let notes = create_doc(
        &s,
        flow.project,
        doc_input(DocumentOwner::Task(flow.task), DocumentKind::Note, "Notes"),
    )
    .await;
    claim(&s, flow.project, flow.task, ClaimPhase::Execute).await;

    let appended = try_append(&s, flow.project, &notes, Some(1), "progress note")
        .await
        .unwrap();
    assert_eq!(appended.number, 2);

    let err = try_append(&s, flow.project, &flow.document, Some(2), "v3")
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::ActiveWork(_)),
        "the selected plan must not change under a claimed execution"
    );
    assert_eq!(revision_count(&s.store, &flow.document).await, 2);
}

// --- Terminal and archived owners ---

#[tokio::test]
async fn terminal_owner_accepts_only_note_documents() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let finding = create_doc(
        &s,
        project,
        doc_input(
            DocumentOwner::Task(task.id),
            DocumentKind::Finding,
            "Finding",
        ),
    )
    .await;
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project,
            task.id,
            "scope change".into(),
        )
        .await
        .unwrap();

    let note = create_doc(
        &s,
        project,
        doc_input(
            DocumentOwner::Task(task.id),
            DocumentKind::Note,
            "Post-mortem",
        ),
    )
    .await;
    let appended = try_append(&s, project, &note, Some(1), "more context")
        .await
        .unwrap();
    assert_eq!(appended.number, 2);

    for kind in [
        DocumentKind::Brief,
        DocumentKind::Plan,
        DocumentKind::Finding,
        DocumentKind::Handoff,
        DocumentKind::Decision,
    ] {
        let err = s
            .store
            .create_document(
                ctx(&s.owner, s.clock.now(), None),
                project,
                doc_input(DocumentOwner::Task(task.id), kind, "Late"),
                "late-hash",
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::TerminalScope),
            "{kind:?} must be rejected on a terminal owner"
        );
    }
    let err = try_append(&s, project, &finding, Some(1), "late output")
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::TerminalScope),
        "terminal outputs cannot be replaced"
    );
}

#[tokio::test]
async fn archived_scope_rejects_document_mutations() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, true, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Plan, "Plan"),
    )
    .await;
    let v1 = document.latest_revision_id.as_uuid();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .cancel_task(
            ctx(&s.owner, s.clock.now(), Some(1)),
            project,
            task.id,
            "obsolete".into(),
        )
        .await
        .unwrap();
    s.clock.advance(TimeDelta::milliseconds(2));
    s.store
        .archive_task(
            ctx(&s.owner, s.clock.now(), Some(2)),
            project,
            task.id,
            "cleanup".into(),
        )
        .await
        .unwrap();

    let err = s
        .store
        .create_document(
            ctx(&s.owner, s.clock.now(), None),
            project,
            doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Late"),
            "late-hash",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = try_append(&s, project, &document, Some(1), "late")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = try_select(&s, project, task.id, v1).await.unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    // Archived documents stay readable.
    assert!(s.store.get_document(&project, &document.id).await.is_ok());
}

// --- Selection validation ---

#[tokio::test]
async fn selection_requires_planning_and_a_task_plan_revision() {
    let s = setup().await;
    let (project, epic, no_planning) = scaffold(&s, false, ReviewPolicy::None).await;
    let plan_doc = create_doc(
        &s,
        project,
        doc_input(
            DocumentOwner::Task(no_planning.id),
            DocumentKind::Plan,
            "Plan",
        ),
    )
    .await;
    let err = try_select(
        &s,
        project,
        no_planning.id,
        plan_doc.latest_revision_id.as_uuid(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "planning_required",
            ..
        }
    ));

    let planning = create_task_full(
        &s,
        project,
        epic,
        "T2",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    // A non-plan document on the task.
    let note = create_doc(
        &s,
        project,
        doc_input(
            DocumentOwner::Task(planning.id),
            DocumentKind::Note,
            "Notes",
        ),
    )
    .await;
    let err = try_select(&s, project, planning.id, note.latest_revision_id.as_uuid())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_id",
            ..
        }
    ));
    // A plan revision owned by a different task.
    let err = try_select(
        &s,
        project,
        planning.id,
        plan_doc.latest_revision_id.as_uuid(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_id",
            ..
        }
    ));
    // A plan revision from another project.
    let foreign = setup_foreign_plan_revision(&s).await;
    let err = try_select(&s, project, planning.id, foreign)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_id",
            ..
        }
    ));
    let (selected, _, _, _) = task_plan_state(&s.store, planning.id).await;
    assert_eq!(
        selected, None,
        "rejected selections leave the task unchanged"
    );
}

async fn setup_foreign_plan_revision(s: &Setup) -> Uuid {
    let project = create_project(s, "Other").await;
    let goal = create_goal(s, project.id, "G").await;
    let epic = create_epic(s, project.id, goal, "E").await;
    let task = create_task_full(
        s,
        project.id,
        epic.id,
        "T",
        true,
        ReviewPolicy::None,
        ReviewPolicy::None,
    )
    .await;
    create_doc(
        s,
        project.id,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Plan, "Plan"),
    )
    .await
    .latest_revision_id
    .as_uuid()
}

// --- Project isolation ---

#[tokio::test]
async fn documents_are_project_isolated() {
    let s = setup().await;
    let (project_a, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let document = create_doc(
        &s,
        project_a,
        doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, "Notes"),
    )
    .await;
    let project_b = create_project(&s, "Other").await.id;

    let err = s
        .store
        .get_document(&project_b, &document.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
    let err = s
        .store
        .get_document_revision(&project_b, &document.latest_revision_id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
    let page = s
        .store
        .list_documents(
            &project_b,
            &shepherd_core::commands::DocumentListFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert!(page.items.is_empty());
    let err = s
        .store
        .list_document_revisions(&project_b, &document.id, &ListParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

// --- Pagination and cursor binding ---

#[tokio::test]
async fn document_lists_paginate_and_bind_cursors() {
    let s = setup().await;
    let (project, _, task) = scaffold(&s, false, ReviewPolicy::None).await;
    let mut ids = Vec::new();
    for title in ["A", "B", "C"] {
        let document = create_doc(
            &s,
            project,
            doc_input(DocumentOwner::Task(task.id), DocumentKind::Note, title),
        )
        .await;
        ids.push(document.id);
    }

    let filters = shepherd_core::commands::DocumentListFilters::default();
    let page_one = s
        .store
        .list_documents(
            &project,
            &filters,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page_one.items.iter().map(|d| d.id).collect::<Vec<_>>(),
        ids[..2],
        "documents list in creation order"
    );
    let cursor = page_one
        .next_cursor
        .expect("three documents exceed one page");
    let page_two = s
        .store
        .list_documents(
            &project,
            &filters,
            &ListParams {
                limit: Some(2),
                cursor: Some(cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_two.items.len(), 1);
    assert_eq!(page_two.items[0].id, ids[2]);
    assert_eq!(page_two.next_cursor, None);

    // The cursor is bound to its filters and endpoint.
    let kind_filtered = shepherd_core::commands::DocumentListFilters {
        kind: Some(DocumentKind::Note),
        ..Default::default()
    };
    let err = s
        .store
        .list_documents(
            &project,
            &kind_filtered,
            &ListParams {
                cursor: Some(cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));
    let err = s
        .store
        .list_document_revisions(
            &project,
            &ids[0],
            &ListParams {
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    // Owner filters narrow the list.
    let epic_only = s
        .store
        .list_documents(
            &project,
            &shepherd_core::commands::DocumentListFilters {
                owner_kind: Some(shepherd_core::model::DocumentOwnerKind::Epic),
                ..Default::default()
            },
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert!(epic_only.items.is_empty());

    // Revision pages walk newest first.
    let document = s.store.get_document(&project, &ids[0]).await.unwrap();
    try_append(&s, project, &document, Some(1), "v2")
        .await
        .unwrap();
    try_append(&s, project, &document, Some(2), "v3")
        .await
        .unwrap();
    let revisions_one = s
        .store
        .list_document_revisions(
            &project,
            &ids[0],
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        revisions_one
            .items
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    let revisions_two = s
        .store
        .list_document_revisions(
            &project,
            &ids[0],
            &ListParams {
                limit: Some(2),
                cursor: revisions_one.next_cursor,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        revisions_two
            .items
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(revisions_two.next_cursor, None);
}

// --- Page-index usage ---

#[tokio::test]
async fn document_lists_use_their_page_indexes_without_a_temp_sort() {
    use sqlx::Row as _;

    let s = setup().await;
    let cases = [
        (
            "EXPLAIN QUERY PLAN SELECT id FROM documents \
             WHERE project_id = 'p' ORDER BY created_at ASC, id ASC LIMIT 51",
            "documents_project_page",
        ),
        // The revisions page is served by the UNIQUE(document_id,number)
        // autoindex; this pins the production query shape.
        (
            "EXPLAIN QUERY PLAN SELECT id FROM document_revisions \
             WHERE document_id = 'd' AND number < 5 ORDER BY number DESC LIMIT 51",
            "sqlite_autoindex_document_revisions",
        ),
    ];
    for (query, index) in cases {
        let rows = sqlx::query(query).fetch_all(s.store.pool()).await.unwrap();
        let plan: Vec<String> = rows
            .iter()
            .map(|row| row.try_get::<String, _>("detail").unwrap())
            .collect();
        let joined = plan.join(" | ");
        assert!(
            joined.contains(index),
            "the page query must use {index}, got: {joined}"
        );
        assert!(
            !joined.to_ascii_lowercase().contains("temp b-tree"),
            "the page query must not sort in a temp B-tree, got: {joined}"
        );
    }
}
