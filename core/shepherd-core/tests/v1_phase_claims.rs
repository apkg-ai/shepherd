use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, ClaimInput, ClaimPhase, ClaimStatus, Clock, CommandId,
    DependencyCreate, Epic, EpicCreate, EpicId, EpicStatus, Goal, GoalCreate, GoalId, Project,
    ProjectCreate, ProjectId, RenewInput, Task, TaskCreate, TaskId, TaskStatus, TestClock,
};
use shepherd_core::storage::rows::insert_actor;
use shepherd_core::storage::{Store, StoreOptions, open, testing};
use uuid::Uuid;

async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(testing::store_options(dir.path(), "shepherd.db", clock))
        .await
        .unwrap()
}

async fn open_independent_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    let opts = StoreOptions {
        db_path: dir.path().join("shepherd.db"),
        mvp_db_path: Some(dir.path().join("mvp").join("shepherd.db")),
        clock,
        codec: Arc::new(shepherd_core::storage::TestCodec::new(Arc::new(
            shepherd_core::storage::TestKeyProvider([3; 32]),
        ))),
    };
    open(opts).await.unwrap()
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

async fn create_task_with_planning(
    s: &Setup,
    project: ProjectId,
    epic: EpicId,
    title: &str,
    planning_required: bool,
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
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
}

async fn create_task(s: &Setup, project: ProjectId, epic: EpicId, title: &str) -> Task {
    create_task_with_planning(s, project, epic, title, false).await
}

fn claim_input(phase: ClaimPhase, ttl: Option<i64>) -> ClaimInput {
    ClaimInput {
        phase,
        submission_id: None,
        ttl_seconds: ttl,
    }
}

async fn claim_status(store: &Store, claim_id: Uuid) -> (String, Option<String>, String) {
    sqlx::query_as("SELECT status, closed_at, close_reason FROM claims WHERE id = ?1")
        .bind(claim_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn count_active_claims(store: &Store, task_id: TaskId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM claims WHERE task_id = ?1 AND status = 'active'")
        .bind(task_id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

async fn reload_task(s: &Setup, project: ProjectId, task_id: TaskId) -> Task {
    s.store.get_task(&project, &task_id).await.unwrap()
}

async fn reload_epic(s: &Setup, project: ProjectId, epic_id: EpicId) -> Epic {
    s.store.get_epic(&project, &epic_id).await.unwrap()
}

// --- Acceptance test 1 ---

#[tokio::test]
async fn ten_simultaneous_claims_yield_one_active_winner() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task_with_planning(&s, project.id, epic.id, "T", true).await;

    // Register 10 agents.
    let mut agents = Vec::new();
    for i in 0..10 {
        s.clock.advance(TimeDelta::milliseconds(1));
        let agent = actor(s.clock.now(), ActorKind::Agent, &format!("agent-{i}"));
        register(&s.store, &agent).await;
        agents.push(agent);
    }

    // Open 10 independent store connections.
    let mut stores = Vec::new();
    for _ in 0..10 {
        stores.push(open_independent_store(&s.dir, s.clock.clone()).await);
    }

    s.clock.advance(TimeDelta::milliseconds(1));
    let now = s.clock.now();

    // Launch 10 concurrent claims.
    let mut handles = Vec::new();
    for (i, store) in stores.into_iter().enumerate() {
        let agent = agents[i].clone();
        let pid = project.id;
        let tid = task.id;
        let rev = task.revision.value();
        handles.push(tokio::spawn(async move {
            store
                .claim_task(
                    CommandContext {
                        actor: agent,
                        command_id: CommandId::generate(now),
                        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
                        expected_revision: Some(rev),
                        now,
                    },
                    pid,
                    tid,
                    claim_input(ClaimPhase::Plan, None),
                    &format!("hash-{i}"),
                )
                .await
        }));
    }

    let mut successes = 0;
    let mut failures = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => successes += 1,
            Err(_) => failures += 1,
        }
    }

    assert_eq!(successes, 1, "exactly one claim must succeed");
    assert_eq!(failures, 9, "nine claims must fail");
    assert_eq!(count_active_claims(&s.store, task.id).await, 1);
}

// --- Acceptance test 2 ---

#[tokio::test]
async fn planning_claim_while_upstream_epic_waits_succeeds_execute_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic_a = create_epic(&s, project.id, goal.id, "Prereq").await;
    let epic_b = create_epic(&s, project.id, goal.id, "Dependent").await;

    // B depends on A.
    s.store
        .create_dependency(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            DependencyCreate::Epic {
                dependent_id: epic_b.id,
                prerequisite_id: epic_a.id,
            },
        )
        .await
        .unwrap();

    let task = create_task_with_planning(&s, project.id, epic_b.id, "Work", true).await;

    s.clock.advance(TimeDelta::milliseconds(1));

    // Planning claim should succeed: can_plan ignores epic prerequisites.
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            claim_input(ClaimPhase::Plan, None),
            "plan-hash",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(grant.claim.phase, ClaimPhase::Plan);
    assert_eq!(grant.claim.status, ClaimStatus::Active);

    // Task should now be active/planning.
    let updated = reload_task(&s, project.id, task.id).await;
    assert_eq!(updated.status, TaskStatus::Active);

    // Epic should still be open (not active until execute).
    let eb = reload_epic(&s, project.id, epic_b.id).await;
    assert_eq!(eb.status, EpicStatus::Open);

    // Release the plan claim.
    s.clock.advance(TimeDelta::milliseconds(1));
    s.store
        .release_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            "release-hash",
        )
        .await
        .unwrap();

    // Execute claim should fail: epic prerequisite unmet.
    let updated = reload_task(&s, project.id, task.id).await;
    s.clock.advance(TimeDelta::milliseconds(1));
    let err = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(updated.revision.value())),
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            "exec-hash",
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, DomainError::NotEligible(_)),
        "expected NotEligible, got: {err:?}"
    );
}

// --- Acceptance test 3 ---

#[tokio::test]
async fn renew_at_exact_expiry_fails() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    s.clock.advance(TimeDelta::milliseconds(1));

    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, Some(60)),
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();

    // Advance by 59 seconds: renew must succeed.
    s.clock.advance(TimeDelta::seconds(59));
    let renewed = s
        .store
        .renew_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            RenewInput {
                ttl_seconds: Some(60),
            },
            "renew-ok-hash",
        )
        .await
        .unwrap()
        .into_inner();
    assert_eq!(renewed.status, ClaimStatus::Active);

    // Advance to exactly expires_at of the renewed claim (now + 60s from renewal).
    s.clock.advance(TimeDelta::seconds(60));

    // At exact expiry: renew must fail (expires_at <= now).
    let err = s
        .store
        .renew_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            RenewInput {
                ttl_seconds: Some(60),
            },
            "renew-fail-hash",
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, DomainError::LeaseInvalid),
        "expected LeaseInvalid at exact expiry, got: {err:?}"
    );
}

// --- Acceptance test 4 ---

#[tokio::test]
async fn block_epic_revokes_planning_and_execution_claims() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task_a = create_task_with_planning(&s, project.id, epic.id, "Plan", true).await;
    let task_b = create_task(&s, project.id, epic.id, "Exec").await;

    s.clock.advance(TimeDelta::milliseconds(1));

    // Claim task A in planning phase.
    let grant_a = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_a.revision.value())),
            project.id,
            task_a.id,
            claim_input(ClaimPhase::Plan, None),
            "plan-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::milliseconds(1));

    // Claim task B in execution phase.
    let task_b_rev = reload_task(&s, project.id, task_b.id).await.revision;
    let grant_b = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_b_rev.value())),
            project.id,
            task_b.id,
            claim_input(ClaimPhase::Execute, None),
            "exec-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::milliseconds(1));

    // Block the epic.
    let epic_rev = reload_epic(&s, project.id, epic.id).await.revision;
    s.store
        .block_epic(
            ctx(&s.owner, s.clock.now(), Some(epic_rev.value())),
            project.id,
            epic.id,
            "upstream issue".to_string(),
        )
        .await
        .unwrap();

    // Both claims should be revoked.
    let (status_a, _, reason_a) = claim_status(&s.store, grant_a.claim.id).await;
    assert_eq!(status_a, "revoked");
    assert_eq!(reason_a, "upstream issue");

    let (status_b, _, reason_b) = claim_status(&s.store, grant_b.claim.id).await;
    assert_eq!(status_b, "revoked");
    assert_eq!(reason_b, "upstream issue");

    // Tasks should still be active (status does not revert on revocation).
    let ta = reload_task(&s, project.id, task_a.id).await;
    assert_eq!(ta.status, TaskStatus::Active);
    let tb = reload_task(&s, project.id, task_b.id).await;
    assert_eq!(tb.status, TaskStatus::Active);

    // No active claims remain.
    assert_eq!(count_active_claims(&s.store, task_a.id).await, 0);
    assert_eq!(count_active_claims(&s.store, task_b.id).await, 0);
}

// --- Acceptance test 5 ---

#[tokio::test]
async fn lost_claim_response_replay_returns_same_lease_release_does_not_erase_saved_output() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    s.clock.advance(TimeDelta::milliseconds(1));

    let idempotency_key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let request_hash = "idempotent-claim-hash";

    let first_ctx = CommandContext {
        actor: s.owner.clone(),
        command_id: CommandId::generate(s.clock.now()),
        idempotency_key,
        expected_revision: Some(task.revision.value()),
        now: s.clock.now(),
    };

    // First claim.
    let first = s
        .store
        .claim_task(
            first_ctx,
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            request_hash,
        )
        .await
        .unwrap();

    assert!(!first.is_replay());
    let grant = first.into_inner();
    let original_token = grant.lease_token.clone();
    let original_claim_id = grant.claim.id;

    // Replay with the same idempotency key and request hash.
    s.clock.advance(TimeDelta::milliseconds(1));
    let replay_ctx = CommandContext {
        actor: s.owner.clone(),
        command_id: CommandId::generate(s.clock.now()),
        idempotency_key,
        expected_revision: Some(task.revision.value()),
        now: s.clock.now(),
    };

    let replayed = s
        .store
        .claim_task(
            replay_ctx,
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            request_hash,
        )
        .await
        .unwrap();

    assert!(replayed.is_replay());
    let replayed_grant = replayed.into_inner();
    assert_eq!(replayed_grant.lease_token, original_token);
    assert_eq!(replayed_grant.claim.id, original_claim_id);

    // Release the claim.
    s.clock.advance(TimeDelta::milliseconds(1));
    let released = s
        .store
        .release_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            original_claim_id,
            &original_token,
            "release-hash",
        )
        .await
        .unwrap()
        .into_inner();
    assert_eq!(released.status, ClaimStatus::Released);

    // Replay the original claim after release: the sealed response must still be available.
    s.clock.advance(TimeDelta::milliseconds(1));
    let post_release_ctx = CommandContext {
        actor: s.owner.clone(),
        command_id: CommandId::generate(s.clock.now()),
        idempotency_key,
        expected_revision: Some(task.revision.value()),
        now: s.clock.now(),
    };

    let post_release = s
        .store
        .claim_task(
            post_release_ctx,
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            request_hash,
        )
        .await
        .unwrap();

    assert!(post_release.is_replay());
    let post_grant = post_release.into_inner();
    assert_eq!(post_grant.lease_token, original_token);
    assert_eq!(post_grant.claim.id, original_claim_id);
}
