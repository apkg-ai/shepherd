use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorId, ActorKind, Claim, ClaimInput, ClaimPhase, ClaimStatus, Clock, CommandId,
    DependencyCreate, DocumentCreate, DocumentKind, DocumentOwner, Epic, EpicCreate, EpicId,
    EpicStatus, Goal, GoalCreate, GoalId, Project, ProjectCreate, ProjectId, RenewInput,
    ReportInput, ReviewPolicy, RevisionCreate, SelectPlanInput, SessionOutcome, Task, TaskCreate,
    TaskId, TaskStatus, TestClock,
};
use shepherd_core::queries::{ClaimListFilters, ListParams, Page};
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

    // The write lock serializes the ten commands: the winner commits first,
    // and every loser then fails the revision check against the bumped task.
    let mut successes = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => successes += 1,
            Err(DomainError::RevisionConflict { .. }) => conflicts += 1,
            Err(other) => panic!("unexpected claim error: {other:?}"),
        }
    }

    assert_eq!(successes, 1, "exactly one claim must succeed");
    assert_eq!(
        conflicts, 9,
        "losers must fail on the winner's revision bump"
    );
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

// --- Input validation ---

#[tokio::test]
async fn review_phase_and_submission_id_are_rejected_until_supported() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    s.clock.advance(TimeDelta::milliseconds(1));

    let err = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Review,
                submission_id: None,
                ttl_seconds: None,
            },
            "review-hash",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::Validation { field: "phase", .. }),
        "expected phase validation, got: {err:?}"
    );

    // plan/04: only review claims reference a submission.
    let err = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project.id,
            task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: Some(Uuid::nil()),
                ttl_seconds: None,
            },
            "submission-hash",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            DomainError::Validation {
                field: "submission_id",
                ..
            }
        ),
        "expected submission_id validation, got: {err:?}"
    );
}

// --- Renewal semantics (plan/04: expiry = now + TTL) ---

#[tokio::test]
async fn renew_resets_the_ttl_from_now_rather_than_extending() {
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
            claim_input(ClaimPhase::Execute, Some(900)),
            "claim-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::seconds(1));
    let renew_at = s.clock.now();
    let renewed = s
        .store
        .renew_claim(
            ctx(&s.owner, renew_at, None),
            project.id,
            grant.claim.id,
            &grant.lease_token,
            RenewInput {
                ttl_seconds: Some(30),
            },
            "renew-hash",
        )
        .await
        .unwrap()
        .into_inner();

    // Renewal resets the lease to now + TTL, even when that truncates a
    // longer remaining window.
    assert_eq!(renewed.expires_at, renew_at + TimeDelta::seconds(30));
    assert!(renewed.expires_at < grant.claim.expires_at);
}

// --- Expiry reconciliation on a later claim ---

#[tokio::test]
async fn claiming_after_expiry_sweeps_the_stale_claim_and_emits_both_events() {
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
            claim_input(ClaimPhase::Execute, Some(30)),
            "first-hash",
        )
        .await
        .unwrap()
        .into_inner();

    // Let the lease lapse. The stale row still says 'active', so the sweep
    // must expire it before the insert can satisfy the one-active-claim
    // partial index.
    s.clock.advance(TimeDelta::seconds(31));
    let updated = reload_task(&s, project.id, task.id).await;
    let regrant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(updated.revision.value())),
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            "second-hash",
        )
        .await
        .unwrap()
        .into_inner();

    let (status, closed_at, reason) = claim_status(&s.store, grant.claim.id).await;
    assert_eq!(status, "expired");
    assert!(closed_at.is_some());
    assert_eq!(reason, "");
    assert_eq!(count_active_claims(&s.store, task.id).await, 1);

    // The swept claim has two claim.changed events (grant + expiry), the
    // replacement exactly one (its grant).
    for (claim_id, expected) in [(grant.claim.id, 2), (regrant.claim.id, 1)] {
        let emitted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE type = 'claim.changed' AND resource_id = ?1",
        )
        .bind(claim_id.to_string())
        .fetch_one(s.store.pool())
        .await
        .unwrap();
        assert_eq!(emitted, expected);
    }
}

// --- Read model: effective expiry, filters, cursor pagination ---

async fn list_page(
    s: &Setup,
    project: ProjectId,
    filters: &ClaimListFilters,
    params: &ListParams,
) -> Page<Claim> {
    s.store
        .list_claims(&project, filters, params)
        .await
        .unwrap()
}

#[tokio::test]
async fn list_claims_computes_effective_expiry_and_honors_filters() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task_a = create_task(&s, project.id, epic.id, "A").await;
    // Planning-required so the plan-phase claim is eligible.
    let task_b = create_task_with_planning(&s, project.id, epic.id, "B", true).await;

    s.clock.advance(TimeDelta::milliseconds(1));
    let grant_a = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_a.revision.value())),
            project.id,
            task_a.id,
            claim_input(ClaimPhase::Execute, None),
            "a-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::milliseconds(1));
    s.store
        .release_claim(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            grant_a.claim.id,
            &grant_a.lease_token,
            "release-a-hash",
        )
        .await
        .unwrap();

    s.clock.advance(TimeDelta::milliseconds(1));
    let grant_b = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task_b.revision.value())),
            project.id,
            task_b.id,
            claim_input(ClaimPhase::Plan, Some(30)),
            "b-hash",
        )
        .await
        .unwrap()
        .into_inner();

    // No write command runs past this point: B's row stays 'active' while
    // its expiry has passed, so only the read model can report it expired.
    s.clock.advance(TimeDelta::seconds(31));
    assert_eq!(claim_status(&s.store, grant_b.claim.id).await.0, "active");

    let empty = ListParams::default();
    let all = list_page(&s, project.id, &ClaimListFilters::default(), &empty).await;
    assert_eq!(all.items.len(), 2);
    // Oldest first (plan/07 list order).
    assert_eq!(all.items[0].id, grant_a.claim.id);
    assert_eq!(all.items[0].status, ClaimStatus::Released);
    assert_eq!(all.items[1].id, grant_b.claim.id);
    assert_eq!(all.items[1].status, ClaimStatus::Expired);

    let expired = list_page(
        &s,
        project.id,
        &ClaimListFilters {
            status: Some(ClaimStatus::Expired),
            ..Default::default()
        },
        &empty,
    )
    .await;
    assert_eq!(expired.items.len(), 1);
    assert_eq!(expired.items[0].id, grant_b.claim.id);

    // Effectively expired rows are not 'active' even though the column says so.
    let active = list_page(
        &s,
        project.id,
        &ClaimListFilters {
            status: Some(ClaimStatus::Active),
            ..Default::default()
        },
        &empty,
    )
    .await;
    assert!(active.items.is_empty());

    let plan_phase = list_page(
        &s,
        project.id,
        &ClaimListFilters {
            phase: Some(ClaimPhase::Plan),
            ..Default::default()
        },
        &empty,
    )
    .await;
    assert_eq!(plan_phase.items.len(), 1);
    assert_eq!(plan_phase.items[0].id, grant_b.claim.id);

    let by_task = list_page(
        &s,
        project.id,
        &ClaimListFilters {
            task_id: Some(task_a.id),
            ..Default::default()
        },
        &empty,
    )
    .await;
    assert_eq!(by_task.items.len(), 1);
    assert_eq!(by_task.items[0].id, grant_a.claim.id);
}

#[tokio::test]
async fn list_claims_paginates_by_acquired_at_without_overlap() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    let task = create_task(&s, project.id, epic.id, "T").await;

    // Three claims on one task: claim, release, claim, release, claim.
    let mut claim_ids = Vec::new();
    for i in 0..3 {
        s.clock.advance(TimeDelta::milliseconds(1));
        let revision = reload_task(&s, project.id, task.id).await.revision;
        let grant = s
            .store
            .claim_task(
                ctx(&s.owner, s.clock.now(), Some(revision.value())),
                project.id,
                task.id,
                claim_input(ClaimPhase::Execute, None),
                &format!("claim-hash-{i}"),
            )
            .await
            .unwrap()
            .into_inner();
        claim_ids.push(grant.claim.id);
        if i < 2 {
            s.clock.advance(TimeDelta::milliseconds(1));
            s.store
                .release_claim(
                    ctx(&s.owner, s.clock.now(), None),
                    project.id,
                    grant.claim.id,
                    &grant.lease_token,
                    &format!("release-hash-{i}"),
                )
                .await
                .unwrap();
        }
    }

    let limited = ListParams {
        limit: Some(2),
        ..Default::default()
    };
    let page_one = list_page(&s, project.id, &ClaimListFilters::default(), &limited).await;
    assert_eq!(page_one.items.len(), 2);
    // Oldest first: the first released claim, then the second released one.
    assert_eq!(page_one.items[0].id, claim_ids[0]);
    assert_eq!(page_one.items[1].id, claim_ids[1]);

    let cursor = page_one.next_cursor.expect("three claims exceed one page");
    let page_two = list_page(
        &s,
        project.id,
        &ClaimListFilters::default(),
        &ListParams {
            limit: Some(2),
            cursor: Some(cursor),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(page_two.items.len(), 1);
    assert_eq!(page_two.items[0].id, claim_ids[2]);
    assert_eq!(page_two.next_cursor, None);
}

// --- Plan revision pinning ---

/// Drives the public plan flow to an accepted two-revision plan with the
/// second revision selected: claim plan, create the plan document, append a
/// second revision, select it, then report plan success (policy none accepts
/// inline and moves the task to execution).
async fn accept_task_plan(s: &Setup, project: ProjectId, task: &Task) -> (Uuid, Uuid) {
    s.clock.advance(TimeDelta::milliseconds(1));
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(task.revision.value())),
            project,
            task.id,
            claim_input(ClaimPhase::Plan, None),
            "plan-claim-hash",
        )
        .await
        .unwrap()
        .into_inner();

    s.clock.advance(TimeDelta::milliseconds(1));
    let document = s
        .store
        .create_document(
            ctx(&s.owner, s.clock.now(), None),
            project,
            DocumentCreate {
                owner: DocumentOwner::Task(task.id),
                kind: DocumentKind::Plan,
                title: "Plan".into(),
                body: String::new(),
                links: Vec::new(),
            },
            "plan-doc-hash",
        )
        .await
        .unwrap()
        .into_inner();
    let first = document.latest_revision_id.as_uuid();

    s.clock.advance(TimeDelta::milliseconds(1));
    let second = s
        .store
        .create_document_revision(
            ctx(&s.owner, s.clock.now(), Some(document.revision.value())),
            project,
            document.id,
            RevisionCreate {
                body: "v2".into(),
                links: Vec::new(),
            },
            "plan-rev-hash",
        )
        .await
        .unwrap()
        .into_inner()
        .id
        .as_uuid();

    s.clock.advance(TimeDelta::milliseconds(1));
    let current = reload_task(s, project, task.id).await;
    s.store
        .select_task_plan(
            ctx(&s.owner, s.clock.now(), Some(current.revision.value())),
            project,
            task.id,
            SelectPlanInput {
                document_revision_id: second,
            },
        )
        .await
        .unwrap();

    s.clock.advance(TimeDelta::milliseconds(1));
    s.store
        .report_claim(
            ctx(&s.owner, s.clock.now(), None),
            project,
            grant.claim.id,
            &grant.lease_token,
            ReportInput {
                outcome: SessionOutcome::Succeeded,
                summary: "planned".into(),
                failure_reason: String::new(),
                document_revision_ids: vec![first, second],
                links: serde_json::json!([]),
            },
            "plan-report-hash",
        )
        .await
        .unwrap();
    (first, second)
}

#[tokio::test]
async fn execute_claim_pins_the_selected_plan_revision() {
    let s = setup().await;
    let project = create_project(&s, "P").await;
    let goal = create_goal(&s, project.id, "G").await;
    let epic = create_epic(&s, project.id, goal.id, "E").await;
    s.clock.advance(TimeDelta::milliseconds(2));
    let task = s
        .store
        .create_task(
            ctx(&s.owner, s.clock.now(), None),
            project.id,
            epic.id,
            TaskCreate {
                title: "T".to_string(),
                type_key: "code".to_string(),
                planning_required: Some(true),
                plan_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    // The accepted plan carries two revisions; the owner selected the second.
    let (first, second) = accept_task_plan(&s, project.id, &task).await;
    let _ = first;

    s.clock.advance(TimeDelta::milliseconds(1));
    let updated = reload_task(&s, project.id, task.id).await;
    let grant = s
        .store
        .claim_task(
            ctx(&s.owner, s.clock.now(), Some(updated.revision.value())),
            project.id,
            task.id,
            claim_input(ClaimPhase::Execute, None),
            "exec-hash",
        )
        .await
        .unwrap()
        .into_inner();

    assert_eq!(grant.claim.plan_revision_ids, vec![second]);
    let pinned: Option<String> =
        sqlx::query_scalar("SELECT plan_revision_id FROM claims WHERE id = ?1")
            .bind(grant.claim.id.to_string())
            .fetch_one(s.store.pool())
            .await
            .unwrap();
    assert_eq!(pinned, Some(second.to_string()));
}
