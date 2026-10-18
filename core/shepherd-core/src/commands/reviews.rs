use uuid::Uuid;

use crate::error::DomainError;
use crate::lease;
use crate::model::{
    Capability, ClaimPhase, ClaimStatus, REASON_MAX_CHARS, Review, ReviewDecision, ReviewInput,
    ReviewPolicy, Submission, SubmissionKind, SubmissionStatus, TaskPhase, require_capability,
    validate_required_text,
};
use crate::queries::hierarchy::task_row;
use crate::queries::submissions::submission_row;
use crate::storage::Store;
use crate::storage::rows::format_ts;
use crate::workflow;
use crate::workflow::eligibility::{GateCode, GateReason, evaluate_task, load_task_snapshots};

use super::claims::{load_claim_row, reconcile_task_claims};
use super::{
    CommandContext, PendingEvent, Replay, append_events, missing_after_write, require_revision,
    task_scope,
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
        if input.reason.chars().count() > REASON_MAX_CHARS {
            return Err(DomainError::Validation {
                field: "reason",
                message: format!("must be at most {REASON_MAX_CHARS} characters"),
            });
        }
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
                    ReviewDecision::Approve => input.reason.clone(),
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
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                             phase = ?2 WHERE id = ?3",
                        )
                        .bind(&now_text)
                        .bind(TaskPhase::Execution.as_str())
                        .bind(task_id.to_string())
                        .execute(&mut **tx)
                        .await?;
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

                sqlx::query(
                    "UPDATE submissions SET status = 'withdrawn', withdraw_reason = ?1, \
                     revision = revision + 1, updated_at = ?2 WHERE id = ?3",
                )
                .bind(&reason)
                .bind(&now_text)
                .bind(submission_id.to_string())
                .execute(&mut **tx)
                .await?;

                // plan/04: plan withdrawal returns planning; work withdrawal
                // returns execution. Selection and acceptance stay untouched.
                let next_phase = match submission.kind {
                    SubmissionKind::Plan => TaskPhase::Planning,
                    SubmissionKind::Work => TaskPhase::Execution,
                };
                sqlx::query(
                    "UPDATE tasks SET revision = revision + 1, updated_at = ?1, phase = ?2 \
                     WHERE id = ?3",
                )
                .bind(&now_text)
                .bind(next_phase.as_str())
                .bind(task_id.to_string())
                .execute(&mut **tx)
                .await?;

                let mut pending = Vec::new();
                pending.push(PendingEvent::submission(
                    submission_id,
                    submission.revision.value() + 1,
                    task_id,
                ));

                // Withdrawal revokes the active review claim; no Review row is
                // fabricated (plan/04).
                let closed = super::revoke_active_claims(
                    tx,
                    super::ClaimScope::Task(task_id),
                    &now_text,
                    &reason,
                )
                .await?;
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
