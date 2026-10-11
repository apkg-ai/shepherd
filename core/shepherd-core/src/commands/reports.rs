use sqlx::SqliteConnection;
use uuid::Uuid;

use crate::error::DomainError;
use crate::lease;
use crate::model::{
    ActorId, ClaimPhase, ClaimStatus, ProjectId, ReportInput, ReviewPolicy, Session,
    SessionOutcome, TaskId, TaskPhase, validate_report_input,
};
use crate::queries::hierarchy::task_row;
use crate::storage::Store;
use crate::storage::rows::format_ts;
use crate::workflow;

use super::claims::{load_claim_row, reconcile_task_claims};
use super::{
    CommandContext, PendingEvent, Replay, append_events, live_actor, missing_after_write,
    task_scope,
};

/// Validate that every UUID in `revision_ids` belongs to a document owned by the task.
async fn validate_document_revisions(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
    revision_ids: &[Uuid],
) -> Result<(), DomainError> {
    if revision_ids.is_empty() {
        return Ok(());
    }
    for id in revision_ids {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT dr.id FROM document_revisions dr \
             JOIN documents d ON d.id = dr.document_id \
             WHERE dr.id = ?1 AND d.owner_kind = 'task' AND d.owner_id = ?2 \
             AND d.project_id = ?3",
        )
        .bind(id.to_string())
        .bind(task.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?;
        if found.is_none() {
            return Err(DomainError::Validation {
                field: "document_revision_ids",
                message: format!("revision {id} does not belong to a document owned by this task"),
            });
        }
    }
    Ok(())
}

impl Store {
    pub async fn report_claim(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        claim_id: Uuid,
        lease_token: &str,
        input: ReportInput,
        request_hash: &str,
    ) -> Result<Replay<Session>, DomainError> {
        let actor_id = ctx.actor.id;
        let now = ctx.now;
        let command_id = ctx.command_id;
        let token = lease_token.to_string();

        #[cfg(any(test, feature = "test-support"))]
        let fp = self
            .report_failpoint
            .load(std::sync::atomic::Ordering::Relaxed);

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                // Step 1: load claim to get task_id.
                let initial = load_claim_row(tx, &claim_id, &project).await?;

                // Step 2: reconcile expired claims on the task.
                let expired = reconcile_task_claims(tx, initial.task_id, &now_text).await?;

                // Step 3: re-load (may now be expired).
                let row = load_claim_row(tx, &claim_id, &project).await?;

                // Step 4: verify lease.
                if row.actor_id != actor_id {
                    return Err(DomainError::LeaseInvalid);
                }
                if !lease::verify_lease(&token, &row.lease_hash) {
                    return Err(DomainError::LeaseInvalid);
                }
                if row.status != ClaimStatus::Active.as_str() {
                    return Err(DomainError::LeaseInvalid);
                }

                // Step 5: load task scope, validate phase alignment.
                validate_report_input(&input, row.phase)?;
                let scope = task_scope(tx, &project, &row.task_id).await?;
                let expected_phase = match row.phase {
                    ClaimPhase::Plan => TaskPhase::Planning,
                    ClaimPhase::Execute => TaskPhase::Execution,
                    ClaimPhase::Review => {
                        return Err(DomainError::Validation {
                            field: "phase",
                            message: "review reports are not yet supported".into(),
                        });
                    }
                };
                if scope.task.phase != expected_phase {
                    return Err(DomainError::InvalidState(format!(
                        "claim phase {} does not match task phase {}",
                        row.phase.as_str(),
                        scope.task.phase.as_str()
                    )));
                }

                // Step 6: validate document revisions for succeeded reports.
                if input.outcome == SessionOutcome::Succeeded {
                    validate_document_revisions(
                        tx,
                        &project,
                        &row.task_id,
                        &input.document_revision_ids,
                    )
                    .await?;
                    if row.phase == ClaimPhase::Execute && input.document_revision_ids.is_empty() {
                        return Err(DomainError::Validation {
                            field: "document_revision_ids",
                            message: "successful execute report requires at least one \
                                      task-owned document revision"
                                .into(),
                        });
                    }
                    if row.phase == ClaimPhase::Plan {
                        let selected = input.selected_plan_revision_id.unwrap();
                        if !input.document_revision_ids.contains(&selected) {
                            return Err(DomainError::Validation {
                                field: "selected_plan_revision_id",
                                message: "must be one of the reported document_revision_ids".into(),
                            });
                        }
                    }
                }

                // Step 7: close claim as reported.
                sqlx::query(
                    "UPDATE claims SET status = 'reported', closed_at = ?1, \
                     close_reason = '' WHERE id = ?2",
                )
                .bind(&now_text)
                .bind(claim_id.to_string())
                .execute(&mut **tx)
                .await?;

                #[cfg(any(test, feature = "test-support"))]
                crate::storage::failpoint::check(fp, crate::storage::failpoint::AFTER_LEASE_CLOSE)?;

                // Step 8: insert immutable session.
                let session_id = crate::model::new_v7(now);
                let doc_rev_json: Vec<String> = input
                    .document_revision_ids
                    .iter()
                    .map(Uuid::to_string)
                    .collect();
                let doc_rev_text = serde_json::to_string(&doc_rev_json)
                    .map_err(|e| crate::storage::StorageError::Corrupt(e.to_string()))?;
                let links_text = serde_json::to_string(&input.links)
                    .map_err(|e| crate::storage::StorageError::Corrupt(e.to_string()))?;

                sqlx::query(
                    "INSERT INTO sessions (id, task_id, claim_id, actor_id, phase, \
                     started_at, ended_at, outcome, summary, failure_reason, \
                     document_revision_ids, links) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )
                .bind(session_id.to_string())
                .bind(row.task_id.to_string())
                .bind(claim_id.to_string())
                .bind(actor_id.to_string())
                .bind(row.phase.as_str())
                .bind(format_ts(&row.acquired_at))
                .bind(&now_text)
                .bind(input.outcome.as_str())
                .bind(&input.summary)
                .bind(&input.failure_reason)
                .bind(&doc_rev_text)
                .bind(&links_text)
                .execute(&mut **tx)
                .await?;

                #[cfg(any(test, feature = "test-support"))]
                crate::storage::failpoint::check(
                    fp,
                    crate::storage::failpoint::AFTER_SESSION_INSERT,
                )?;

                // Step 9: determine phase transition, create submission, update task.
                let mut pending = Vec::new();
                let context_revision = scope.task.revision.value();
                let mut submission_id: Option<Uuid> = None;

                for closed in &expired {
                    pending.push(PendingEvent::claim(
                        closed.id,
                        closed.task_revision,
                        closed.task_id,
                    ));
                }

                match (row.phase, input.outcome) {
                    (ClaimPhase::Plan, SessionOutcome::Failed) => {
                        // No phase change, no attempt_count change.
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1 \
                             WHERE id = ?2",
                        )
                        .bind(&now_text)
                        .bind(row.task_id.to_string())
                        .execute(&mut **tx)
                        .await?;
                    }
                    (ClaimPhase::Plan, SessionOutcome::Succeeded) => {
                        let sub_id = crate::model::new_v7(now);
                        let review_policy = scope.task.plan_review;
                        let sub_status = if review_policy == ReviewPolicy::None {
                            "accepted"
                        } else {
                            "pending"
                        };
                        let next_phase = if review_policy == ReviewPolicy::None {
                            TaskPhase::Execution
                        } else {
                            TaskPhase::PlanReview
                        };
                        let selected = input.selected_plan_revision_id.unwrap();

                        insert_submission(
                            tx,
                            &SubmissionRow {
                                id: sub_id,
                                now: &now_text,
                                task_id: row.task_id,
                                kind: "plan",
                                producer_id: actor_id,
                                document_revision_ids_json: &doc_rev_text,
                                session_id,
                                policy: review_policy,
                                status: sub_status,
                                context_revision,
                            },
                        )
                        .await?;

                        if review_policy == ReviewPolicy::None {
                            sqlx::query(
                                "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                                 phase = ?2, selected_plan_revision_id = ?3, \
                                 accepted_plan_submission_id = ?4 WHERE id = ?5",
                            )
                            .bind(&now_text)
                            .bind(next_phase.as_str())
                            .bind(selected.to_string())
                            .bind(sub_id.to_string())
                            .bind(row.task_id.to_string())
                            .execute(&mut **tx)
                            .await?;
                        } else {
                            sqlx::query(
                                "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                                 phase = ?2, selected_plan_revision_id = ?3 WHERE id = ?4",
                            )
                            .bind(&now_text)
                            .bind(next_phase.as_str())
                            .bind(selected.to_string())
                            .bind(row.task_id.to_string())
                            .execute(&mut **tx)
                            .await?;
                        }

                        submission_id = Some(sub_id);
                    }
                    (ClaimPhase::Execute, SessionOutcome::Failed) => {
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                             attempt_count = attempt_count + 1 WHERE id = ?2",
                        )
                        .bind(&now_text)
                        .bind(row.task_id.to_string())
                        .execute(&mut **tx)
                        .await?;
                    }
                    (ClaimPhase::Execute, SessionOutcome::Succeeded) => {
                        let sub_id = crate::model::new_v7(now);
                        let review_policy = scope.task.work_review;
                        let sub_status = if review_policy == ReviewPolicy::None {
                            "accepted"
                        } else {
                            "pending"
                        };

                        insert_submission(
                            tx,
                            &SubmissionRow {
                                id: sub_id,
                                now: &now_text,
                                task_id: row.task_id,
                                kind: "work",
                                producer_id: actor_id,
                                document_revision_ids_json: &doc_rev_text,
                                session_id,
                                policy: review_policy,
                                status: sub_status,
                                context_revision,
                            },
                        )
                        .await?;

                        if review_policy == ReviewPolicy::None {
                            // Increment attempt_count, then use complete() for
                            // the done/complete transition and its PendingEvent.
                            sqlx::query(
                                "UPDATE tasks SET attempt_count = attempt_count + 1 \
                                 WHERE id = ?1",
                            )
                            .bind(row.task_id.to_string())
                            .execute(&mut **tx)
                            .await?;
                            let refreshed = task_row(tx, &project, &row.task_id)
                                .await?
                                .ok_or_else(|| missing_after_write("task"))?;
                            let actor = live_actor(tx, &actor_id).await?;
                            let complete_ctx = CommandContext {
                                actor,
                                command_id,
                                idempotency_key: Uuid::nil(),
                                expected_revision: None,
                                now,
                            };
                            workflow::task::complete(tx, &complete_ctx, &refreshed, &mut pending)
                                .await?;
                        } else {
                            sqlx::query(
                                "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                                 phase = ?2, attempt_count = attempt_count + 1 WHERE id = ?3",
                            )
                            .bind(&now_text)
                            .bind(TaskPhase::WorkReview.as_str())
                            .bind(row.task_id.to_string())
                            .execute(&mut **tx)
                            .await?;
                        }

                        submission_id = Some(sub_id);
                    }
                    (ClaimPhase::Review, _) => unreachable!(),
                }

                #[cfg(any(test, feature = "test-support"))]
                crate::storage::failpoint::check(fp, crate::storage::failpoint::AFTER_TASK_UPDATE)?;

                // Step 10: cascade (execute + done + review=none).
                if row.phase == ClaimPhase::Execute
                    && input.outcome == SessionOutcome::Succeeded
                    && scope.task.work_review == ReviewPolicy::None
                {
                    workflow::recompute(
                        tx,
                        workflow::AffectedScope {
                            project,
                            epics: vec![scope.task.epic_id],
                        },
                        &mut pending,
                        now,
                    )
                    .await?;
                }

                // Step 11: collect events.
                // For execute+done, complete() already pushed a task event.
                // For all other paths, push it now.
                let needs_task_event = !(row.phase == ClaimPhase::Execute
                    && input.outcome == SessionOutcome::Succeeded
                    && scope.task.work_review == ReviewPolicy::None);
                if needs_task_event {
                    let updated = task_row(tx, &project, &row.task_id)
                        .await?
                        .ok_or_else(|| missing_after_write("task"))?;
                    pending.push(PendingEvent::task(
                        row.task_id,
                        updated.revision,
                        scope.task.epic_id,
                    ));
                }

                pending.push(PendingEvent::claim(
                    claim_id,
                    row.task_revision,
                    row.task_id,
                ));
                if let Some(sub_id) = submission_id {
                    pending.push(PendingEvent::submission(sub_id, 1, row.task_id));
                }
                pending.push(PendingEvent::session(session_id, row.task_id));

                let actor = live_actor(tx, &actor_id).await?;
                let events_ctx = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                append_events(tx, &project, &events_ctx, "reportClaim", "", pending).await?;

                #[cfg(any(test, feature = "test-support"))]
                crate::storage::failpoint::check(
                    fp,
                    crate::storage::failpoint::AFTER_EVENT_INSERT,
                )?;

                Ok(Session {
                    id: session_id,
                    task_id: row.task_id,
                    claim_id,
                    actor_id,
                    phase: row.phase,
                    started_at: row.acquired_at,
                    ended_at: now,
                    outcome: input.outcome,
                    summary: input.summary,
                    failure_reason: input.failure_reason,
                    document_revision_ids: input.document_revision_ids,
                    links: input.links,
                })
            })
        })
        .await
    }
}

struct SubmissionRow<'a> {
    id: Uuid,
    now: &'a str,
    task_id: TaskId,
    kind: &'a str,
    producer_id: ActorId,
    document_revision_ids_json: &'a str,
    session_id: Uuid,
    policy: ReviewPolicy,
    status: &'a str,
    context_revision: i64,
}

async fn insert_submission(
    conn: &mut SqliteConnection,
    row: &SubmissionRow<'_>,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO submissions (id, revision, created_at, updated_at, task_id, kind, \
         producer_id, document_revision_ids, session_id, policy, status, \
         created_context_revision) \
         VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(row.id.to_string())
    .bind(row.now)
    .bind(row.task_id.to_string())
    .bind(row.kind)
    .bind(row.producer_id.to_string())
    .bind(row.document_revision_ids_json)
    .bind(row.session_id.to_string())
    .bind(row.policy.as_str())
    .bind(row.status)
    .bind(row.context_revision)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
