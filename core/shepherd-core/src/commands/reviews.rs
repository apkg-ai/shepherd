use uuid::Uuid;

use crate::error::DomainError;
use crate::lease;
use crate::model::{
    Capability, ClaimPhase, ClaimStatus, REASON_MAX_CHARS, Review, ReviewDecision, ReviewInput,
    ReviewPolicy, Submission, SubmissionKind, SubmissionStatus, TaskId, TaskPhase,
    require_capability, validate_long_text, validate_required_text,
};
use crate::queries::hierarchy::task_row;
use crate::queries::submissions::submission_row;
use crate::storage::Store;
use crate::storage::rows::format_ts;
use crate::workflow;
use crate::workflow::eligibility::{GateCode, GateReason, evaluate_task, load_task_snapshots};

use super::claims::{load_claim_row, reconcile_task_claims};
use super::{
    ClaimScope, CommandContext, PendingEvent, Replay, append_events, missing_after_write,
    require_revision, revoke_active_claims, task_scope, withdraw_pending_submissions,
};

// plan/04: human-policy and self-review violations are 403, never a
// retryable eligibility gate.
pub(super) fn review_gate_error(reasons: &[GateReason]) -> DomainError {
    if reasons
        .iter()
        .any(|reason| reason.code == GateCode::ProducerCannotReview)
    {
        return DomainError::ProducerCannotReview;
    }
    if let Some(reason) = reasons
        .iter()
        .find(|reason| reason.code == GateCode::ReviewerPolicy)
    {
        return DomainError::ReviewerPolicy(reason.message.clone());
    }
    if reasons.is_empty() {
        return DomainError::NotEligible("no pending submission awaits review".into());
    }
    let joined: Vec<&str> = reasons.iter().map(|reason| reason.code.as_str()).collect();
    DomainError::NotEligible(joined.join(", "))
}

fn require_pending(submission: &Submission) -> Result<(), DomainError> {
    if submission.status != SubmissionStatus::Pending {
        return Err(DomainError::SubmissionSuperseded(format!(
            "submission is {}",
            submission.status.as_str()
        )));
    }
    Ok(())
}

// plan/04: reject-work and withdraw perform the same phase return.
async fn set_task_phase(
    tx: &mut sqlx::SqliteConnection,
    now_text: &str,
    task_id: TaskId,
    phase: TaskPhase,
) -> Result<(), DomainError> {
    sqlx::query(
        "UPDATE tasks SET revision = revision + 1, updated_at = ?1, phase = ?2 WHERE id = ?3",
    )
    .bind(now_text)
    .bind(phase.as_str())
    .bind(task_id.to_string())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

impl Store {
    pub async fn review_submission(
        &self,
        ctx: CommandContext,
        project: crate::model::ProjectId,
        submission_id: Uuid,
        lease_token: Option<&str>,
        input: ReviewInput,
        request_hash: &str,
    ) -> Result<Replay<Review>, DomainError> {
        let actor_id = ctx.actor.id;
        let expected_revision = ctx.expected_revision;
        let now = ctx.now;
        let command_id = ctx.command_id;
        let token = lease_token.map(str::to_string);

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let submission = submission_row(tx, &project, &submission_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let task_id = submission.task_id;
                let scope = task_scope(tx, &project, &task_id).await?;
                let actor = super::live_actor(tx, &actor_id).await?;

                match submission.policy {
                    ReviewPolicy::Human => {
                        require_capability(&actor, Capability::ReviewHumanPolicy)?
                    }
                    ReviewPolicy::Agent => {
                        require_capability(&actor, Capability::ReviewAgentPolicy)?
                    }
                    // Unreachable via public flow: none-policy submissions are
                    // accepted at report time and never pending.
                    ReviewPolicy::None => {
                        return Err(DomainError::ReviewerPolicy(
                            "policy none has no reviewer".into(),
                        ));
                    }
                }
                require_revision(expected_revision, submission.revision)?;

                // Agent review rides the exclusive review claim; human review
                // is claimless (plan/04).
                let review_claim = match submission.policy {
                    ReviewPolicy::Agent => {
                        let claim_id = input.claim_id.ok_or(DomainError::Validation {
                            field: "claim_id",
                            message: "agent reviews require the review claim".into(),
                        })?;
                        let token = token.ok_or(DomainError::LeaseInvalid)?;
                        reconcile_task_claims(tx, task_id, &now_text).await?;
                        let row = load_claim_row(tx, &claim_id, &project)
                            .await
                            .map_err(|err| match err {
                                DomainError::NotFound => DomainError::LeaseInvalid,
                                other => other,
                            })?;
                        if row.actor_id != actor_id
                            || !lease::verify_lease(&token, &row.lease_hash)
                            || row.status != ClaimStatus::Active.as_str()
                            || row.phase != ClaimPhase::Review
                            || row.submission_id != Some(submission_id)
                        {
                            return Err(DomainError::LeaseInvalid);
                        }
                        if actor_id == submission.producer_id {
                            return Err(DomainError::ProducerCannotReview);
                        }
                        Some(row)
                    }
                    _ => {
                        if input.claim_id.is_some() {
                            return Err(DomainError::Validation {
                                field: "claim_id",
                                message: "human policy reviews are claimless".into(),
                            });
                        }
                        None
                    }
                };

                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                require_pending(&submission)?;

                // The agent path was authorized at claim acquisition; human
                // review re-evaluates eligibility directly (plan/04).
                if submission.policy == ReviewPolicy::Human {
                    let snapshots = load_task_snapshots(tx, &project, &[task_id]).await?;
                    let snapshot = snapshots
                        .get(&task_id)
                        .ok_or_else(|| missing_after_write("task snapshot"))?;
                    let eligibility = evaluate_task(snapshot, &actor, now);
                    if !eligibility.can_review {
                        return Err(review_gate_error(&eligibility.reasons));
                    }
                }

                let reason = match input.decision {
                    ReviewDecision::Reject => {
                        validate_required_text("reason", &input.reason, REASON_MAX_CHARS)?
                    }
                    ReviewDecision::Approve => {
                        validate_long_text("reason", &input.reason, REASON_MAX_CHARS)?;
                        input.reason.clone()
                    }
                };

                let review_id = crate::model::new_v7(now);
                sqlx::query(
                    "INSERT INTO reviews (id, submission_id, actor_id, decision, reason, \
                     created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .bind(review_id.to_string())
                .bind(submission_id.to_string())
                .bind(actor_id.to_string())
                .bind(input.decision.as_str())
                .bind(&reason)
                .bind(&now_text)
                .execute(&mut **tx)
                .await?;

                let next_status = match input.decision {
                    ReviewDecision::Approve => SubmissionStatus::Accepted,
                    ReviewDecision::Reject => SubmissionStatus::Rejected,
                };
                sqlx::query(
                    "UPDATE submissions SET status = ?1, revision = revision + 1, \
                     updated_at = ?2 WHERE id = ?3",
                )
                .bind(next_status.as_str())
                .bind(&now_text)
                .bind(submission_id.to_string())
                .execute(&mut **tx)
                .await?;

                let mut pending = Vec::new();
                pending.push(PendingEvent::review(review_id, submission_id, task_id));
                pending.push(PendingEvent::submission(
                    submission_id,
                    submission.revision.value() + 1,
                    task_id,
                ));

                let events_ctx = CommandContext::for_events(actor, command_id, now);
                let epic_id = scope.task.epic_id;
                let mut did_complete = false;
                match (submission.kind, input.decision) {
                    (SubmissionKind::Plan, ReviewDecision::Approve) => {
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                             phase = ?2, accepted_plan_submission_id = ?3 WHERE id = ?4",
                        )
                        .bind(&now_text)
                        .bind(TaskPhase::Execution.as_str())
                        .bind(submission_id.to_string())
                        .bind(task_id.to_string())
                        .execute(&mut **tx)
                        .await?;
                    }
                    // plan/04 "reject plan: clear acceptance": both the
                    // acceptance pointer and the stale selection are cleared.
                    (SubmissionKind::Plan, ReviewDecision::Reject) => {
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                             phase = ?2, accepted_plan_submission_id = NULL, \
                             selected_plan_revision_id = NULL WHERE id = ?3",
                        )
                        .bind(&now_text)
                        .bind(TaskPhase::Planning.as_str())
                        .bind(task_id.to_string())
                        .execute(&mut **tx)
                        .await?;
                    }
                    (SubmissionKind::Work, ReviewDecision::Approve) => {
                        workflow::task::complete(tx, &events_ctx, &scope.task, &mut pending)
                            .await?;
                        did_complete = true;
                    }
                    // Plan retained: selection and acceptance stay untouched.
                    (SubmissionKind::Work, ReviewDecision::Reject) => {
                        set_task_phase(tx, &now_text, task_id, TaskPhase::Execution).await?;
                    }
                }

                if let Some(claim) = &review_claim {
                    sqlx::query("UPDATE claims SET status = ?1, closed_at = ?2 WHERE id = ?3")
                        .bind(ClaimStatus::Released.as_str())
                        .bind(&now_text)
                        .bind(claim.id.to_string())
                        .execute(&mut **tx)
                        .await?;
                    pending.push(PendingEvent::claim(claim.id, claim.task_revision, task_id));
                }

                if did_complete {
                    workflow::recompute(
                        tx,
                        workflow::AffectedScope {
                            project,
                            epics: vec![epic_id],
                        },
                        &mut pending,
                        now,
                    )
                    .await?;
                } else {
                    let updated = task_row(tx, &project, &task_id)
                        .await?
                        .ok_or_else(|| missing_after_write("task"))?;
                    pending.push(PendingEvent::task(task_id, updated.revision, epic_id));
                }

                append_events(
                    tx,
                    &project,
                    &events_ctx,
                    "reviewSubmission",
                    &reason,
                    pending,
                )
                .await?;

                Ok(Review {
                    id: review_id,
                    submission_id,
                    actor_id,
                    decision: input.decision,
                    reason,
                    created_at: now,
                })
            })
        })
        .await
    }

    pub async fn withdraw_submission(
        &self,
        ctx: CommandContext,
        project: crate::model::ProjectId,
        submission_id: Uuid,
        reason: String,
        request_hash: &str,
    ) -> Result<Replay<Submission>, DomainError> {
        let actor_id = ctx.actor.id;
        let expected_revision = ctx.expected_revision;
        let now = ctx.now;
        let command_id = ctx.command_id;

        self.idempotent_transaction(&ctx, request_hash, 200, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let submission = submission_row(tx, &project, &submission_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let task_id = submission.task_id;
                let scope = task_scope(tx, &project, &task_id).await?;
                let actor = super::live_actor(tx, &actor_id).await?;

                require_capability(&actor, Capability::WithdrawSubmission)?;
                // plan/12: the owner or the producer may withdraw; other
                // agents may not.
                if actor.kind != crate::model::ActorKind::Human
                    && actor_id != submission.producer_id
                {
                    return Err(DomainError::Forbidden(
                        "only the owner or the producer may withdraw a submission".into(),
                    ));
                }
                require_revision(expected_revision, submission.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                require_pending(&submission)?;
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;

                // one_pending_submission_per_task: the shared task-scoped
                // sweep hits exactly this submission.
                let withdrawn_row =
                    withdraw_pending_submissions(tx, ClaimScope::Task(task_id), &now_text, &reason)
                        .await?
                        .into_iter()
                        .find(|row| row.id == submission_id)
                        .ok_or_else(|| missing_after_write("pending submission"))?;

                // plan/04: plan withdrawal returns planning; work withdrawal
                // returns execution. Selection and acceptance stay untouched.
                let next_phase = match submission.kind {
                    SubmissionKind::Plan => TaskPhase::Planning,
                    SubmissionKind::Work => TaskPhase::Execution,
                };
                set_task_phase(tx, &now_text, task_id, next_phase).await?;

                let mut pending = Vec::new();
                pending.push(PendingEvent::submission(
                    submission_id,
                    withdrawn_row.revision,
                    task_id,
                ));

                // Withdrawal revokes the active review claim; no Review row is
                // fabricated (plan/04).
                let closed =
                    revoke_active_claims(tx, ClaimScope::Task(task_id), &now_text, &reason).await?;
                for claim in closed.all() {
                    pending.push(PendingEvent::claim(
                        claim.id,
                        claim.task_revision,
                        claim.task_id,
                    ));
                }

                let epic_id = scope.task.epic_id;
                let updated = task_row(tx, &project, &task_id)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;
                pending.push(PendingEvent::task(task_id, updated.revision, epic_id));

                let events_ctx = CommandContext::for_events(actor, command_id, now);
                append_events(
                    tx,
                    &project,
                    &events_ctx,
                    "withdrawSubmission",
                    &reason,
                    pending,
                )
                .await?;

                let withdrawn = submission_row(tx, &project, &submission_id)
                    .await?
                    .ok_or_else(|| missing_after_write("submission"))?;
                Ok(withdrawn)
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use super::*;
    use crate::model::{
        Actor, ActorKind, ClaimInput, Clock, CommandId, DocumentKind, EpicCreate, GoalCreate,
        ProjectCreate, ProjectId, ReportInput, SelectPlanInput, SessionOutcome, Task, TaskCreate,
        TaskId, TestClock,
    };
    use crate::queries::{ListParams, SubmissionListFilters};
    use crate::storage::open;
    use crate::storage::testing::{seed_actor, store_options, test_clock};
    use crate::workflow::eligibility::GateReason;
    use uuid::Uuid;

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        producer: Actor,
        project: ProjectId,
        task: Task,
    }

    fn ctx(actor: &Actor, clock: &TestClock, revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            request_hash: Some("test-request".into()),
            expected_revision: revision,
            now: clock.now(),
        }
    }

    async fn fixture(
        planning_required: bool,
        plan_review: ReviewPolicy,
        work_review: ReviewPolicy,
    ) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = seed_actor(&store, ActorKind::Human, "owner").await;
        let producer = seed_actor(&store, ActorKind::Agent, "producer").await;
        let project = store
            .create_project(
                ctx(&owner, &clock, None),
                ProjectCreate {
                    name: "P".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let goal = store
            .create_goal(
                ctx(&owner, &clock, None),
                project,
                GoalCreate {
                    title: "G".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let epic = store
            .create_epic(
                ctx(&owner, &clock, None),
                project,
                goal,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let task = store
            .create_task(
                ctx(&owner, &clock, None),
                project,
                epic,
                TaskCreate {
                    title: "T".to_string(),
                    description: None,
                    type_key: "code".to_string(),
                    planning_required: Some(planning_required),
                    plan_review: Some(plan_review),
                    work_review: Some(work_review),
                },
            )
            .await
            .unwrap()
            .value;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            producer,
            project,
            task,
        }
    }

    async fn task_revision(f: &Fixture) -> i64 {
        sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
            .bind(f.task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap()
    }

    /// (phase, selected_plan_revision_id, accepted_plan_submission_id, status)
    async fn task_state(f: &Fixture) -> (String, Option<String>, Option<String>, String) {
        sqlx::query_as(
            "SELECT phase, selected_plan_revision_id, accepted_plan_submission_id, status \
             FROM tasks WHERE id = ?1",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap()
    }

    async fn claim_as(
        f: &Fixture,
        who: &Actor,
        phase: ClaimPhase,
        submission_id: Option<Uuid>,
    ) -> (Uuid, String) {
        f.clock.advance(TimeDelta::milliseconds(2));
        let revision = task_revision(f).await;
        let grant = f
            .store
            .claim_task(
                ctx(who, &f.clock, Some(revision)),
                f.project,
                f.task.id,
                ClaimInput {
                    phase,
                    submission_id,
                    ttl_seconds: None,
                },
                "claim-hash",
            )
            .await
            .unwrap()
            .into_inner();
        (grant.claim.id, grant.lease_token)
    }

    async fn report_success(f: &Fixture, claim_id: Uuid, token: &str, revisions: Vec<Uuid>) {
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .report_claim(
                ctx(&f.producer, &f.clock, None),
                f.project,
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

    async fn doc_revision(f: &Fixture, kind: DocumentKind) -> Uuid {
        f.clock.advance(TimeDelta::milliseconds(2));
        crate::storage::testing::create_task_document(
            &f.store, &f.owner, f.project, f.task.id, kind,
        )
        .await
        .latest_revision_id
        .as_uuid()
    }

    async fn pending_submission(f: &Fixture) -> Uuid {
        let id: String = sqlx::query_scalar(
            "SELECT id FROM submissions WHERE task_id = ?1 AND status = 'pending'",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        Uuid::parse_str(&id).unwrap()
    }

    /// Plan flow to a pending (or auto-accepted) plan submission.
    async fn run_plan_flow(f: &Fixture) {
        let (claim_id, token) = claim_as(f, &f.producer, ClaimPhase::Plan, None).await;
        let revision = doc_revision(f, DocumentKind::Plan).await;
        f.clock.advance(TimeDelta::milliseconds(2));
        let task_rev = task_revision(f).await;
        f.store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(task_rev)),
                f.project,
                f.task.id,
                SelectPlanInput {
                    document_revision_id: revision,
                },
            )
            .await
            .unwrap();
        report_success(f, claim_id, &token, vec![revision]).await;
    }

    /// Execute flow to a pending work submission.
    async fn run_work_flow(f: &Fixture) {
        let (claim_id, token) = claim_as(f, &f.producer, ClaimPhase::Execute, None).await;
        let revision = doc_revision(f, DocumentKind::Finding).await;
        report_success(f, claim_id, &token, vec![revision]).await;
    }

    async fn decide(
        f: &Fixture,
        who: &Actor,
        submission: Uuid,
        revision: i64,
        decision: ReviewDecision,
        reason: &str,
        claim: Option<(Uuid, &str)>,
    ) -> Result<Review, DomainError> {
        f.clock.advance(TimeDelta::milliseconds(2));
        let (claim_id, token) = match claim {
            Some((id, token)) => (Some(id), Some(token)),
            None => (None, None),
        };
        f.store
            .review_submission(
                ctx(who, &f.clock, Some(revision)),
                f.project,
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
            .map(Replay::into_inner)
    }

    #[test]
    fn gate_error_maps_the_review_reasons() {
        let producer = GateReason {
            code: GateCode::ProducerCannotReview,
            resource_id: Uuid::nil(),
            message: "producer".into(),
        };
        let policy = GateReason {
            code: GateCode::ReviewerPolicy,
            resource_id: Uuid::nil(),
            message: "policy".into(),
        };
        let claimed = GateReason {
            code: GateCode::Claimed,
            resource_id: Uuid::nil(),
            message: "claimed".into(),
        };
        assert!(matches!(
            review_gate_error(&[policy.clone(), producer]),
            DomainError::ProducerCannotReview
        ));
        assert!(matches!(
            review_gate_error(&[claimed.clone(), policy]),
            DomainError::ReviewerPolicy(_)
        ));
        assert!(matches!(
            review_gate_error(&[claimed]),
            DomainError::NotEligible(_)
        ));
        assert!(matches!(
            review_gate_error(&[]),
            DomainError::NotEligible(_)
        ));
    }

    #[tokio::test]
    async fn human_plan_review_approves_into_execution() {
        let f = fixture(true, ReviewPolicy::Human, ReviewPolicy::None).await;
        run_plan_flow(&f).await;
        let submission = pending_submission(&f).await;

        let review = decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            None,
        )
        .await
        .unwrap();
        assert!(matches!(review.decision, ReviewDecision::Approve));

        let (phase, selected, accepted, _) = task_state(&f).await;
        assert_eq!(phase, "execution");
        assert!(selected.is_some());
        assert_eq!(accepted, Some(submission.to_string()));
    }

    #[tokio::test]
    async fn human_plan_review_rejects_back_to_planning() {
        let f = fixture(true, ReviewPolicy::Human, ReviewPolicy::None).await;
        run_plan_flow(&f).await;
        let submission = pending_submission(&f).await;

        decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Reject,
            "thin plan",
            None,
        )
        .await
        .unwrap();

        let (phase, selected, accepted, _) = task_state(&f).await;
        assert_eq!(phase, "planning");
        assert_eq!(selected, None);
        assert_eq!(accepted, None);
    }

    #[tokio::test]
    async fn human_work_review_approves_to_done_and_rejects_to_execution() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::Human).await;
        run_work_flow(&f).await;
        let first = pending_submission(&f).await;
        decide(&f, &f.owner, first, 1, ReviewDecision::Reject, "redo", None)
            .await
            .unwrap();
        let (phase, _, _, status) = task_state(&f).await;
        assert_eq!(phase, "execution");
        assert_eq!(status, "active");

        run_work_flow(&f).await;
        let second = pending_submission(&f).await;
        decide(&f, &f.owner, second, 1, ReviewDecision::Approve, "", None)
            .await
            .unwrap();
        let (phase, _, _, status) = task_state(&f).await;
        assert_eq!(phase, "complete");
        assert_eq!(status, "done");
    }

    #[tokio::test]
    async fn agent_review_decides_with_lease_and_releases_claim() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::Agent).await;
        run_work_flow(&f).await;
        let submission = pending_submission(&f).await;
        let reviewer = seed_actor(&f.store, ActorKind::Agent, "reviewer").await;
        let (claim_id, token) = claim_as(&f, &reviewer, ClaimPhase::Review, Some(submission)).await;

        // Wrong token, then missing claim, then the real decision.
        let err = decide(
            &f,
            &reviewer,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            Some((claim_id, "forged")),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::LeaseInvalid));
        let err = decide(
            &f,
            &reviewer,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "claim_id",
                ..
            }
        ));

        decide(
            &f,
            &reviewer,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            Some((claim_id, &token)),
        )
        .await
        .unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM claims WHERE id = ?1")
            .bind(claim_id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(status, "released");
    }

    #[tokio::test]
    async fn decision_shape_errors_are_rejected() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::Human).await;
        run_work_flow(&f).await;
        let submission = pending_submission(&f).await;

        let err = decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Reject,
            " ",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ));
        let oversize = "x".repeat(REASON_MAX_CHARS + 1);
        let err = decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Reject,
            &oversize,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ));
        let err = decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            Some((Uuid::nil(), "token")),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "claim_id",
                ..
            }
        ));
        let err = decide(
            &f,
            &f.producer,
            submission,
            1,
            ReviewDecision::Approve,
            "",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
        let err = decide(
            &f,
            &f.owner,
            Uuid::nil(),
            1,
            ReviewDecision::Approve,
            "",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        decide(
            &f,
            &f.owner,
            submission,
            1,
            ReviewDecision::Reject,
            "redo",
            None,
        )
        .await
        .unwrap();
        let err = decide(
            &f,
            &f.owner,
            submission,
            2,
            ReviewDecision::Approve,
            "",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::SubmissionSuperseded(_)));
    }

    #[tokio::test]
    async fn withdrawal_routes_phase_and_guards_actors() {
        let f = fixture(true, ReviewPolicy::Human, ReviewPolicy::None).await;
        run_plan_flow(&f).await;
        let submission = pending_submission(&f).await;
        let bystander = seed_actor(&f.store, ActorKind::Agent, "bystander").await;

        f.clock.advance(TimeDelta::milliseconds(2));
        let err = f
            .store
            .withdraw_submission(
                ctx(&bystander, &f.clock, Some(1)),
                f.project,
                submission,
                "mine".to_string(),
                "withdraw-hash",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        f.clock.advance(TimeDelta::milliseconds(2));
        let err = f
            .store
            .withdraw_submission(
                ctx(&f.producer, &f.clock, Some(1)),
                f.project,
                submission,
                "  ".to_string(),
                "withdraw-hash",
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ));

        f.clock.advance(TimeDelta::milliseconds(2));
        let withdrawn = f
            .store
            .withdraw_submission(
                ctx(&f.producer, &f.clock, Some(1)),
                f.project,
                submission,
                "found a gap".to_string(),
                "withdraw-hash",
            )
            .await
            .unwrap()
            .into_inner();
        assert_eq!(withdrawn.status, SubmissionStatus::Withdrawn);
        let (phase, ..) = task_state(&f).await;
        assert_eq!(phase, "planning");

        f.clock.advance(TimeDelta::milliseconds(2));
        let err = f
            .store
            .withdraw_submission(
                ctx(&f.owner, &f.clock, Some(2)),
                f.project,
                submission,
                "again".to_string(),
                "withdraw-hash",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::SubmissionSuperseded(_)));
    }

    #[tokio::test]
    async fn submission_reads_list_get_and_reviews() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::Human).await;
        run_work_flow(&f).await;
        let first = pending_submission(&f).await;
        decide(&f, &f.owner, first, 1, ReviewDecision::Reject, "redo", None)
            .await
            .unwrap();
        run_work_flow(&f).await;
        let second = pending_submission(&f).await;

        let all = f
            .store
            .list_submissions(
                &f.project,
                &SubmissionListFilters::default(),
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(all.items.len(), 2);
        assert_eq!(all.items[0].id, second, "newest first");
        assert!(all.next_cursor.is_none());

        let filtered = f
            .store
            .list_submissions(
                &f.project,
                &SubmissionListFilters {
                    task_id: Some(f.task.id),
                    status: Some(SubmissionStatus::Rejected),
                    kind: Some(SubmissionKind::Work),
                    policy: Some(ReviewPolicy::Human),
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].id, first);

        let page_one = f
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
        let cursor = page_one.next_cursor.unwrap();
        let page_two = f
            .store
            .list_submissions(
                &f.project,
                &SubmissionListFilters::default(),
                &ListParams {
                    limit: Some(1),
                    cursor: Some(cursor),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page_two.items[0].id, first);
        let err = f
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
        assert!(matches!(err, DomainError::InvalidCursor(_)));
        let err = f
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

        let fetched = f.store.get_submission(&f.project, &first).await.unwrap();
        assert_eq!(fetched.status, SubmissionStatus::Rejected);
        assert_eq!(fetched.producer_id, f.producer.id);
        let err = f
            .store
            .get_submission(&f.project, &Uuid::nil())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        let reviews = f
            .store
            .list_submission_reviews(&f.project, &first, &ListParams::default())
            .await
            .unwrap();
        assert_eq!(reviews.items.len(), 1);
        assert!(matches!(reviews.items[0].decision, ReviewDecision::Reject));
        assert_eq!(reviews.items[0].reason, "redo");
        let err = f
            .store
            .list_submission_reviews(&f.project, &Uuid::nil(), &ListParams::default())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }
}
