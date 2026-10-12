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
use crate::storage::rows::{format_ts, parse_uuid};
use crate::workflow;

use super::claims::{load_claim_row, reconcile_task_claims};
use super::content::validate_task_owned_revisions;
use super::{CommandContext, PendingEvent, Replay, append_events, missing_after_write, task_scope};

/// Submissions are accepted inline under policy none, pending otherwise.
fn submission_status(policy: ReviewPolicy) -> &'static str {
    if policy == ReviewPolicy::None {
        "accepted"
    } else {
        "pending"
    }
}

async fn selected_plan_revision(
    conn: &mut SqliteConnection,
    task: TaskId,
) -> Result<Option<Uuid>, DomainError> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT selected_plan_revision_id FROM tasks WHERE id = ?1")
            .bind(task.to_string())
            .fetch_one(&mut *conn)
            .await?;
    stored
        .map(|value| parse_uuid("tasks.selected_plan_revision_id", &value))
        .transpose()
        .map_err(DomainError::from)
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
        let actor = ctx.actor.clone();

        #[cfg(any(test, feature = "test-support"))]
        let fp = self
            .report_failpoint
            .load(std::sync::atomic::Ordering::Relaxed);

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let mut input = input;
                let now_text = format_ts(&now);

                let initial = load_claim_row(tx, &claim_id, &project).await?;
                // one_active_claim_per_task: this reconcile can only expire the
                // reported claim itself, making the reloaded row reject it.
                reconcile_task_claims(tx, initial.task_id, &now_text).await?;
                // Re-load: the claim may have been reconciled to expired.
                let row = load_claim_row(tx, &claim_id, &project).await?;

                if row.actor_id != actor_id {
                    return Err(DomainError::LeaseInvalid);
                }
                if !lease::verify_lease(&token, &row.lease_hash) {
                    return Err(DomainError::LeaseInvalid);
                }
                if row.status != ClaimStatus::Active.as_str() {
                    return Err(DomainError::LeaseInvalid);
                }

                validate_report_input(&input)?;
                let scope = task_scope(tx, &project, &row.task_id).await?;
                scope.ensure_unarchived()?;
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

                // Duplicates pass the wire contract but must not reach the
                // session row or the ownership query.
                let mut seen = std::collections::HashSet::new();
                input.document_revision_ids.retain(|id| seen.insert(*id));
                validate_task_owned_revisions(
                    tx,
                    &project,
                    &row.task_id,
                    &input.document_revision_ids,
                )
                .await?;

                if input.outcome == SessionOutcome::Succeeded {
                    if row.phase == ClaimPhase::Execute && input.document_revision_ids.is_empty() {
                        return Err(DomainError::Validation {
                            field: "document_revision_ids",
                            message: "successful execute report requires at least one \
                                      task-owned document revision"
                                .into(),
                        });
                    }
                    if row.phase == ClaimPhase::Plan {
                        // selectTaskPlan is the only writer of this column and
                        // already enforces plan-kind task ownership.
                        let selected = selected_plan_revision(tx, row.task_id).await?;
                        let Some(selected) = selected else {
                            return Err(DomainError::Validation {
                                field: "selected_plan_revision_id",
                                message: "successful plan report requires a selected task plan"
                                    .into(),
                            });
                        };
                        if !input.document_revision_ids.contains(&selected) {
                            return Err(DomainError::Validation {
                                field: "document_revision_ids",
                                message: "must include the selected plan revision".into(),
                            });
                        }
                    }
                }

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

                let mut pending = Vec::new();
                let context_revision = scope.task.revision.value();
                let mut submission_id: Option<Uuid> = None;
                let mut did_complete = false;

                // idempotent_transaction re-checks the actor inside this
                // transaction, so the context snapshot is safe here.
                let events_ctx = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };

                match (row.phase, input.outcome) {
                    (ClaimPhase::Plan, SessionOutcome::Failed) => {
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
                        let sub_status = submission_status(review_policy);
                        let next_phase = if review_policy == ReviewPolicy::None {
                            TaskPhase::Execution
                        } else {
                            TaskPhase::PlanReview
                        };

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

                        // Policy none accepts inline; the accepted submission id
                        // is only written for that policy. The selection itself
                        // was persisted earlier by selectTaskPlan.
                        sqlx::query(
                            "UPDATE tasks SET revision = revision + 1, updated_at = ?1, \
                             phase = ?2, accepted_plan_submission_id = ?3 WHERE id = ?4",
                        )
                        .bind(&now_text)
                        .bind(next_phase.as_str())
                        .bind((review_policy == ReviewPolicy::None).then_some(sub_id.to_string()))
                        .bind(row.task_id.to_string())
                        .execute(&mut **tx)
                        .await?;

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
                        let sub_status = submission_status(review_policy);

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
                            // Increment attempt_count separately; complete()
                            // owns the done/complete transition and revision bump.
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
                            workflow::task::complete(tx, &events_ctx, &refreshed, &mut pending)
                                .await?;
                            did_complete = true;
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
                    (ClaimPhase::Review, _) => {
                        // Defense in depth: the expected_phase guard above
                        // already rejects this; reordering must not panic.
                        return Err(DomainError::Validation {
                            field: "phase",
                            message: "review reports are not yet supported".into(),
                        });
                    }
                }

                #[cfg(any(test, feature = "test-support"))]
                crate::storage::failpoint::check(fp, crate::storage::failpoint::AFTER_TASK_UPDATE)?;

                if did_complete {
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

                // complete() already pushed a task event; all other paths push here.
                if !did_complete {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use super::*;
    use crate::model::{
        Actor, ActorKind, Clock, CommandId, EpicCreate, GoalCreate, ProjectCreate, Task,
        TaskCreate, TaskStatus, TestClock,
    };
    use crate::model::{DocumentCreate, DocumentKind, DocumentOwner, SelectPlanInput};
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{seed_actor, store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        task: Task,
        plan_rev: Uuid,
        finding_rev: Uuid,
    }

    fn ctx(actor: &Actor, clock: &TestClock, revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
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
        let owner = Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind: ActorKind::Human,
            label: "owner".to_string(),
            revoked: false,
            created_at: clock.now(),
        };
        let registered = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();
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
        let plan_rev =
            create_task_doc(&store, &owner, &clock, project, task.id, DocumentKind::Plan).await;
        let finding_rev = create_task_doc(
            &store,
            &owner,
            &clock,
            project,
            task.id,
            DocumentKind::Finding,
        )
        .await;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            task,
            plan_rev,
            finding_rev,
        }
    }

    async fn create_task_doc(
        store: &Store,
        owner: &Actor,
        clock: &TestClock,
        project: ProjectId,
        task: TaskId,
        kind: DocumentKind,
    ) -> Uuid {
        store
            .create_document(
                ctx(owner, clock, None),
                project,
                DocumentCreate {
                    owner: DocumentOwner::Task(task),
                    kind,
                    title: format!("{} doc", kind.as_str()),
                    body: String::new(),
                    links: Vec::new(),
                },
                "doc-hash",
            )
            .await
            .unwrap()
            .into_inner()
            .latest_revision_id
            .as_uuid()
    }

    async fn select_plan(f: &Fixture, revision: Uuid) {
        let (task_revision, _, _, _) = task_state(f).await;
        f.store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(task_revision)),
                f.project,
                f.task.id,
                SelectPlanInput {
                    document_revision_id: revision,
                },
            )
            .await
            .unwrap();
    }

    async fn claim(f: &Fixture, phase: ClaimPhase) -> (Uuid, String) {
        let grant = f
            .store
            .claim_task(
                ctx(&f.owner, &f.clock, Some(f.task.revision.value())),
                f.project,
                f.task.id,
                crate::model::ClaimInput {
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

    fn success_input(revisions: Vec<Uuid>) -> ReportInput {
        ReportInput {
            outcome: SessionOutcome::Succeeded,
            summary: "done".into(),
            failure_reason: String::new(),
            document_revision_ids: revisions,
            links: serde_json::json!([]),
        }
    }

    fn failed_input() -> ReportInput {
        ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "attempt failed".into(),
            failure_reason: "boom".into(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
        }
    }

    async fn report(
        f: &Fixture,
        actor: &Actor,
        claim_id: Uuid,
        token: &str,
        input: ReportInput,
    ) -> Result<Replay<Session>, DomainError> {
        f.store
            .report_claim(
                ctx(actor, &f.clock, None),
                f.project,
                claim_id,
                token,
                input,
                "report-hash",
            )
            .await
    }

    async fn task_state(f: &Fixture) -> (i64, String, String, i64) {
        let row: (i64, String, String, i64) = sqlx::query_as(
            "SELECT revision, status, phase, attempt_count FROM tasks WHERE id = ?1",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        row
    }

    async fn claim_status(f: &Fixture, claim_id: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM claims WHERE id = ?1")
            .bind(claim_id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap()
    }

    async fn session_revisions(f: &Fixture) -> Vec<String> {
        let text: String =
            sqlx::query_scalar("SELECT document_revision_ids FROM sessions WHERE task_id = ?1")
                .bind(f.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    async fn report_event_types(f: &Fixture) -> Vec<String> {
        sqlx::query_scalar("SELECT type FROM events WHERE action = 'reportClaim' ORDER BY id")
            .fetch_all(f.store.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn execute_report_success_completes_task_and_cascades() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        let session = report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap()
        .into_inner();
        assert_eq!(session.outcome, SessionOutcome::Succeeded);
        assert_eq!(claim_status(&f, claim_id).await, "reported");
        let (revision, status, phase, attempts) = task_state(&f).await;
        assert_eq!(status, TaskStatus::Done.as_str());
        assert_eq!(phase, TaskPhase::Complete.as_str());
        assert_eq!(attempts, 1);
        assert!(revision > f.task.revision.value());
        let (kind, sub_status): (String, String) =
            sqlx::query_as("SELECT kind, status FROM submissions WHERE task_id = ?1")
                .bind(f.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!((kind.as_str(), sub_status.as_str()), ("work", "accepted"));
        let epic_status: String = sqlx::query_scalar(
            "SELECT status FROM epics WHERE id = (SELECT epic_id FROM tasks WHERE id = ?1)",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(epic_status, "done");
        let events = report_event_types(&f).await;
        assert!(events.contains(&"task.changed".to_string()));
        assert!(events.contains(&"claim.changed".to_string()));
        assert!(events.contains(&"submission.changed".to_string()));
        assert!(events.contains(&"session.recorded".to_string()));
        assert!(events.contains(&"epic.changed".to_string()));
    }

    #[tokio::test]
    async fn execute_report_success_with_review_pends_work_submission() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::Human).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap();
        let (_, status, phase, attempts) = task_state(&f).await;
        assert_eq!(phase, TaskPhase::WorkReview.as_str());
        assert_eq!(status, TaskStatus::Active.as_str());
        assert_eq!(attempts, 1);
        let (kind, sub_status, policy): (String, String, String) =
            sqlx::query_as("SELECT kind, status, policy FROM submissions WHERE task_id = ?1")
                .bind(f.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(
            (kind.as_str(), sub_status.as_str(), policy.as_str()),
            ("work", "pending", "human")
        );
    }

    #[tokio::test]
    async fn execute_report_failed_counts_attempt_and_keeps_phase() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        report(&f, &f.owner, claim_id, &token, failed_input())
            .await
            .unwrap();
        let (revision, status, phase, attempts) = task_state(&f).await;
        assert_eq!(phase, TaskPhase::Execution.as_str());
        assert_eq!(status, TaskStatus::Active.as_str());
        assert_eq!(attempts, 1);
        assert!(revision > f.task.revision.value());
        assert_eq!(claim_status(&f, claim_id).await, "reported");
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(sessions, 1);
    }

    #[tokio::test]
    async fn plan_report_failed_bumps_revision_without_attempt() {
        let f = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Plan).await;
        report(&f, &f.owner, claim_id, &token, failed_input())
            .await
            .unwrap();
        let (revision, status, phase, attempts) = task_state(&f).await;
        assert_eq!(phase, TaskPhase::Planning.as_str());
        assert_eq!(status, TaskStatus::Active.as_str());
        assert_eq!(attempts, 0);
        assert!(revision > 2);
        let submissions: i64 = sqlx::query_scalar("SELECT count(*) FROM submissions")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(submissions, 0);
    }

    #[tokio::test]
    async fn plan_report_success_without_review_accepts_submission() {
        let f = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Plan).await;
        select_plan(&f, f.plan_rev).await;
        report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.plan_rev]),
        )
        .await
        .unwrap();
        let (_, status, phase, attempts) = task_state(&f).await;
        assert_eq!(phase, TaskPhase::Execution.as_str());
        assert_eq!(status, TaskStatus::Active.as_str());
        assert_eq!(attempts, 0);
        let (selected, accepted): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT selected_plan_revision_id, accepted_plan_submission_id \
             FROM tasks WHERE id = ?1",
        )
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(selected.as_deref(), Some(f.plan_rev.to_string().as_str()));
        assert!(accepted.is_some());
        let (kind, sub_status): (String, String) =
            sqlx::query_as("SELECT kind, status FROM submissions WHERE task_id = ?1")
                .bind(f.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!((kind.as_str(), sub_status.as_str()), ("plan", "accepted"));
    }

    #[tokio::test]
    async fn plan_report_success_with_review_pends_submission() {
        let f = fixture(true, ReviewPolicy::Human, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Plan).await;
        select_plan(&f, f.plan_rev).await;
        report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.plan_rev]),
        )
        .await
        .unwrap();
        let (_, status, phase, _) = task_state(&f).await;
        assert_eq!(phase, TaskPhase::PlanReview.as_str());
        assert_eq!(status, TaskStatus::Active.as_str());
        let (kind, sub_status, policy): (String, String, String) =
            sqlx::query_as("SELECT kind, status, policy FROM submissions WHERE task_id = ?1")
                .bind(f.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(
            (kind.as_str(), sub_status.as_str(), policy.as_str()),
            ("plan", "pending", "human")
        );
    }

    #[tokio::test]
    async fn plan_report_success_requires_prior_selection() {
        let f = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Plan).await;
        let err = report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.plan_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "selected_plan_revision_id",
                ..
            }
        ));
        assert_eq!(claim_status(&f, claim_id).await, "active");
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    }

    #[tokio::test]
    async fn duplicate_revision_ids_are_deduplicated() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.finding_rev, f.finding_rev, f.finding_rev]),
        )
        .await
        .unwrap();
        assert_eq!(session_revisions(&f).await, vec![f.finding_rev.to_string()]);
    }

    #[tokio::test]
    async fn foreign_revision_ids_are_rejected_for_both_outcomes() {
        for (outcome, failure_reason) in [
            (SessionOutcome::Succeeded, String::new()),
            (SessionOutcome::Failed, "boom".to_string()),
        ] {
            let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
            let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
            let other = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
            let input = ReportInput {
                outcome,
                summary: "s".into(),
                failure_reason,
                document_revision_ids: vec![other.finding_rev],
                links: serde_json::json!([]),
            };
            let err = report(&f, &f.owner, claim_id, &token, input)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    DomainError::Validation {
                        field: "document_revision_ids",
                        ..
                    }
                ),
                "outcome {outcome:?} must reject foreign revisions"
            );
            assert_eq!(claim_status(&f, claim_id).await, "active");
        }
    }

    #[tokio::test]
    async fn expired_claim_report_is_lease_invalid_and_reconciles() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        f.clock.advance(TimeDelta::seconds(1_000));
        let err = report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::LeaseInvalid));
        // The rejection rolls the reconciliation back with the transaction.
        assert_eq!(claim_status(&f, claim_id).await, "active");
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    }

    #[tokio::test]
    async fn wrong_actor_and_wrong_token_reports_are_lease_invalid() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        let other = seed_actor(&f.store, ActorKind::Agent, "other").await;
        let err = report(
            &f,
            &other,
            claim_id,
            &token,
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::LeaseInvalid));
        let err = report(
            &f,
            &f.owner,
            claim_id,
            "wrong-token",
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::LeaseInvalid));
        assert_eq!(claim_status(&f, claim_id).await, "active");
    }

    #[tokio::test]
    async fn phase_mismatch_is_invalid_state() {
        let f = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
        // A claim whose phase cannot match the task phase; the lease hash must
        // verify so the mismatch guard is what rejects the report.
        let claim_id = crate::model::new_v7(f.clock.now());
        let hash = crate::lease::lease_hash("seed-token");
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
             status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'execute', ?4, ?5, \
             'active', ?6, ?7)",
        )
        .bind(claim_id.to_string())
        .bind(f.task.id.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(format_ts(&(f.clock.now() + TimeDelta::seconds(300))))
        .bind(1)
        .bind(&hash)
        .execute(f.store.pool())
        .await
        .unwrap();
        let err = report(
            &f,
            &f.owner,
            claim_id,
            "seed-token",
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
        assert_eq!(claim_status(&f, claim_id).await, "active");
    }

    #[tokio::test]
    async fn execute_report_success_requires_a_revision() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Execute).await;
        let err = report(&f, &f.owner, claim_id, &token, success_input(Vec::new()))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "document_revision_ids",
                ..
            }
        ));
        assert_eq!(claim_status(&f, claim_id).await, "active");
    }

    #[tokio::test]
    async fn plan_report_selection_must_be_among_reported_revisions() {
        let f = fixture(true, ReviewPolicy::None, ReviewPolicy::None).await;
        let (claim_id, token) = claim(&f, ClaimPhase::Plan).await;
        select_plan(&f, f.plan_rev).await;
        let err = report(
            &f,
            &f.owner,
            claim_id,
            &token,
            success_input(vec![f.finding_rev]),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "document_revision_ids",
                ..
            }
        ));
        assert_eq!(claim_status(&f, claim_id).await, "active");
    }

    #[tokio::test]
    async fn review_claim_report_is_unsupported() {
        let f = fixture(false, ReviewPolicy::None, ReviewPolicy::None).await;
        let claim_id = crate::model::new_v7(f.clock.now());
        let hash = crate::lease::lease_hash("seed-token");
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
             status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'review', ?4, ?5, \
             'active', ?6, ?7)",
        )
        .bind(claim_id.to_string())
        .bind(f.task.id.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(format_ts(&(f.clock.now() + TimeDelta::seconds(300))))
        .bind(1)
        .bind(&hash)
        .execute(f.store.pool())
        .await
        .unwrap();
        let err = report(&f, &f.owner, claim_id, "seed-token", failed_input())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "phase", .. }
        ));
    }
}
