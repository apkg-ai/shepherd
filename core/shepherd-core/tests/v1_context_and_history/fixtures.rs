use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimGrant, ClaimInput, ClaimPhase, Clock, CommandId, Document,
    DocumentCreate, DocumentKind, DocumentOwner, DocumentRevision, Epic, EpicCreate, EpicId,
    GoalCreate, GoalId, Project, ProjectCreate, ProjectId, ReportInput, ReviewDecision,
    ReviewInput, ReviewPolicy, RevisionCreate, SelectPlanInput, SessionOutcome, Task, TaskCreate,
    TaskId, TestClock, TextPatch,
};
use shepherd_core::queries::ListParams;
use shepherd_core::storage::rows::insert_actor;
use shepherd_core::storage::{Store, open, testing};
use uuid::Uuid;

pub async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(testing::store_options(dir.path(), "shepherd.db", clock))
        .await
        .unwrap()
}

pub fn actor(now: DateTime<Utc>, kind: ActorKind, label: &str) -> Actor {
    Actor {
        id: ActorId::generate(now),
        kind,
        label: label.to_string(),
        revoked: false,
        created_at: now,
    }
}

pub async fn register(store: &Store, actor: &Actor) {
    let inserted = actor.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
        .await
        .unwrap();
}

pub fn ctx(actor: &Actor, now: DateTime<Utc>, expected_revision: Option<i64>) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        request_hash: Some("test-request".into()),
        expected_revision,
        now,
    }
}

pub fn ctx_with_key(actor: &Actor, now: DateTime<Utc>, key: Uuid) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: key,
        request_hash: None,
        expected_revision: None,
        now,
    }
}

pub struct Setup {
    pub dir: tempfile::TempDir,
    pub clock: Arc<TestClock>,
    pub store: Store,
    pub owner: Actor,
}

pub async fn setup() -> Setup {
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

pub async fn agent(s: &Setup, label: &str) -> Actor {
    s.clock.advance(TimeDelta::milliseconds(2));
    let agent = actor(s.clock.now(), ActorKind::Agent, label);
    register(&s.store, &agent).await;
    agent
}

pub async fn create_project(s: &Setup, name: &str) -> Project {
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

pub async fn create_goal(s: &Setup, project: ProjectId, title: &str) -> GoalId {
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

pub async fn create_epic(s: &Setup, project: ProjectId, goal: GoalId, title: &str) -> Epic {
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

pub async fn create_task_full(
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
pub async fn scaffold(
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

pub async fn create_document(
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

pub async fn add_revision(
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

pub async fn claim(
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
pub async fn report(
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

pub async fn select_plan(s: &Setup, project: ProjectId, task: TaskId, revision_id: Uuid) {
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

pub async fn review(
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

pub async fn task_revision(store: &Store, task: TaskId) -> i64 {
    sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
        .bind(task.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

pub async fn submission_revision(store: &Store, submission: Uuid) -> i64 {
    sqlx::query_scalar("SELECT revision FROM submissions WHERE id = ?1")
        .bind(submission.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

pub async fn pending_submission_id(store: &Store, task: TaskId) -> Uuid {
    let id: String =
        sqlx::query_scalar("SELECT id FROM submissions WHERE task_id = ?1 AND status = 'pending'")
            .bind(task.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    Uuid::parse_str(&id).unwrap()
}

pub async fn event_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

pub async fn last_event_id(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// (id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, action, reason)
pub type EventRow = (
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

pub async fn events_after(store: &Store, marker: i64) -> Vec<EventRow> {
    sqlx::query_as(
        "SELECT id, type, resource_id, resource_revision, command_id, actor_id, occurred_at, \
         action, reason FROM events WHERE id > ?1 ORDER BY id ASC",
    )
    .bind(marker)
    .fetch_all(store.pool())
    .await
    .unwrap()
}

pub fn kind_rank(event_type: &str) -> u8 {
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
pub fn assert_group(rows: &[EventRow], required: &[&str], exact: bool) {
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

pub fn history_params(limit: Option<u32>, cursor: Option<String>) -> ListParams {
    ListParams {
        limit,
        cursor,
        include_archived: false,
    }
}

// --- Workflow fixtures ---

pub struct AcceptedPlan {
    pub project: ProjectId,
    pub task: Task,
    pub plan_document: Document,
    pub first_revision: Uuid,
}

/// Plan flow to acceptance under the human gate, with a second plan revision
/// appended so the document's latest differs from the pinned selection.
pub async fn accepted_plan_with_newer_revision(s: &Setup) -> AcceptedPlan {
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
pub async fn failed_attempts(
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

pub fn context_paths(project: ProjectId, epic: EpicId, task: TaskId) -> [String; 6] {
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

/// Scaffold plus enough goal updates to cross the default page size.
pub async fn seeded_history(s: &Setup) -> (ProjectId, GoalId) {
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

/// Raw synthetic event rows for backlog bounds; commands cannot mint thousands
/// of events deterministically fast.
pub async fn seed_events(s: &Setup, project: ProjectId, count: usize) {
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
