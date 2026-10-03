use chrono::TimeDelta;
use sqlx::SqliteConnection;
use uuid::Uuid;

use crate::error::DomainError;
use crate::lease;
use crate::model::{
    ActorId, Capability, Claim, ClaimGrant, ClaimInput, ClaimPhase, ClaimStatus, EpicStatus,
    ProjectId, RenewInput, TaskId, require_capability, validate_ttl,
};
use crate::queries::hierarchy::{epic_row, task_row};
use crate::storage::Store;
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::workflow::eligibility::{evaluate_task, load_task_snapshots};

use super::{
    ClosedClaim, CommandContext, PendingEvent, Replay, append_events, missing_after_write,
    require_revision, task_scope,
};

struct ClaimRow {
    id: Uuid,
    task_id: TaskId,
    actor_id: ActorId,
    phase: ClaimPhase,
    submission_id: Option<Uuid>,
    acquired_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    status: String,
    task_revision: i64,
    plan_revision_id: Option<Uuid>,
    lease_hash: String,
}

async fn load_claim_row(
    conn: &mut SqliteConnection,
    claim_id: &Uuid,
    project: &ProjectId,
) -> Result<ClaimRow, DomainError> {
    let row: Option<ClaimDbRow> = sqlx::query_as(
        "SELECT c.id, c.task_id, c.actor_id, c.phase, c.submission_id, \
         c.acquired_at, c.expires_at, c.status, c.task_revision, c.plan_revision_id, \
         c.lease_hash \
         FROM claims c \
         JOIN tasks t ON t.id = c.task_id \
         WHERE c.id = ?1 AND t.project_id = ?2",
    )
    .bind(claim_id.to_string())
    .bind(project.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    let row = row.ok_or(DomainError::NotFound)?;
    Ok(ClaimRow {
        id: parse_uuid("claims.id", &row.0)?,
        task_id: TaskId::from_uuid(parse_uuid("claims.task_id", &row.1)?),
        actor_id: ActorId::from_uuid(parse_uuid("claims.actor_id", &row.2)?),
        phase: ClaimPhase::parse(&row.3).ok_or_else(|| {
            DomainError::Storage(crate::storage::StorageError::Corrupt(format!(
                "invalid claim phase: {}",
                row.3
            )))
        })?,
        submission_id: row
            .4
            .as_deref()
            .map(|s| parse_uuid("claims.submission_id", s))
            .transpose()?,
        acquired_at: parse_ts("claims.acquired_at", &row.5)?,
        expires_at: parse_ts("claims.expires_at", &row.6)?,
        status: row.7.clone(),
        task_revision: row.8,
        plan_revision_id: row
            .9
            .as_deref()
            .map(|s| parse_uuid("claims.plan_revision_id", s))
            .transpose()?,
        lease_hash: row.10.clone(),
    })
}

type ClaimDbRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    i64,
    Option<String>,
    String,
);

fn claim_from_row(row: &ClaimRow, status_override: ClaimStatus) -> Claim {
    Claim {
        id: row.id,
        task_id: row.task_id,
        actor_id: row.actor_id,
        phase: row.phase,
        submission_ids: row.submission_id.into_iter().collect(),
        acquired_at: row.acquired_at,
        expires_at: row.expires_at,
        status: status_override,
        task_revision: row.task_revision,
        plan_revision_ids: row.plan_revision_id.into_iter().collect(),
    }
}

async fn reconcile_task_claims(
    conn: &mut SqliteConnection,
    task_id: TaskId,
    now: &str,
) -> Result<Vec<ClosedClaim>, DomainError> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "UPDATE claims SET status = 'expired', closed_at = ?2, close_reason = '' \
         WHERE status = 'active' AND expires_at <= ?2 AND task_id = ?1 \
         RETURNING id, task_id, task_revision",
    )
    .bind(task_id.to_string())
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|(id, tid, task_revision)| {
            Ok(ClosedClaim {
                id: parse_uuid("claims.id", &id)?,
                task_id: TaskId::from_uuid(parse_uuid("claims.task_id", &tid)?),
                task_revision,
            })
        })
        .collect()
}

impl Store {
    pub async fn claim_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task_id: TaskId,
        input: ClaimInput,
        request_hash: &str,
    ) -> Result<Replay<ClaimGrant>, DomainError> {
        if input.phase == ClaimPhase::Review {
            return Err(DomainError::Validation {
                field: "phase",
                message: "review claims are not yet supported".into(),
            });
        }
        let ttl = validate_ttl(input.ttl_seconds)?;
        let phase = input.phase;
        let actor_id = ctx.actor.id;
        let expected_revision = ctx.expected_revision;
        let now = ctx.now;
        let command_id = ctx.command_id;

        self.idempotent_transaction(&ctx, request_hash, 201, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let expired = reconcile_task_claims(tx, task_id, &now_text).await?;

                let scope = task_scope(tx, &project, &task_id).await?;
                let actor = super::live_actor(tx, &actor_id).await?;
                require_capability(&actor, Capability::ClaimWork)?;
                require_revision(expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }

                let snapshots = load_task_snapshots(tx, &project, &[task_id]).await?;
                let snapshot = snapshots
                    .get(&task_id)
                    .ok_or_else(|| missing_after_write("task snapshot"))?;
                let eligibility = evaluate_task(snapshot, &actor, now);

                let eligible = match phase {
                    ClaimPhase::Plan => eligibility.can_plan,
                    ClaimPhase::Execute => eligibility.can_execute,
                    ClaimPhase::Review => unreachable!(),
                };
                if !eligible {
                    let reasons: Vec<&str> = eligibility
                        .reasons
                        .iter()
                        .map(|r| r.code.as_str())
                        .collect();
                    return Err(DomainError::NotEligible(reasons.join(", ")));
                }

                let lease_token = lease::generate_lease_token();
                let hash = lease::lease_hash(&lease_token);

                let plan_revision_id: Option<Uuid> = if phase == ClaimPhase::Execute {
                    snapshot.accepted_plan_revision_ids.first().copied()
                } else {
                    None
                };

                let claim_id = crate::model::new_v7(now);
                let expires_at = now + TimeDelta::seconds(ttl);
                let expires_text = format_ts(&expires_at);
                let task_rev = scope.task.revision.value();

                let submission_id_str = input.submission_id.map(|id| id.to_string());

                sqlx::query(
                    "INSERT INTO claims (id, task_id, actor_id, phase, submission_id, \
                     acquired_at, expires_at, status, task_revision, plan_revision_id, \
                     lease_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?10)",
                )
                .bind(claim_id.to_string())
                .bind(task_id.to_string())
                .bind(actor_id.to_string())
                .bind(phase.as_str())
                .bind(&submission_id_str)
                .bind(&now_text)
                .bind(&expires_text)
                .bind(task_rev)
                .bind(plan_revision_id.map(|id| id.to_string()))
                .bind(&hash)
                .execute(&mut **tx)
                .await?;

                sqlx::query(
                    "UPDATE tasks SET status = 'active', revision = revision + 1, \
                     updated_at = ?1 WHERE id = ?2",
                )
                .bind(&now_text)
                .bind(task_id.to_string())
                .execute(&mut **tx)
                .await?;

                let updated_task = task_row(tx, &project, &task_id)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;

                let mut pending = Vec::new();

                for closed in &expired {
                    pending.push(PendingEvent::claim(
                        closed.id,
                        closed.task_revision,
                        closed.task_id,
                    ));
                }

                let epic_id = scope.epic.id;
                let goal_id = scope.epic.goal_id;
                if phase == ClaimPhase::Execute && scope.epic.status == EpicStatus::Open {
                    sqlx::query(
                        "UPDATE epics SET status = 'active', revision = revision + 1, \
                         updated_at = ?1 WHERE id = ?2 AND status = 'open'",
                    )
                    .bind(&now_text)
                    .bind(epic_id.to_string())
                    .execute(&mut **tx)
                    .await?;
                    let updated_epic = epic_row(tx, &project, &epic_id)
                        .await?
                        .ok_or_else(|| missing_after_write("epic"))?;
                    pending.push(PendingEvent::epic(epic_id, updated_epic.revision, goal_id));
                }

                pending.push(PendingEvent::task(task_id, updated_task.revision, epic_id));
                pending.push(PendingEvent::claim(claim_id, task_rev, task_id));

                let ctx_for_events = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                append_events(tx, &project, &ctx_for_events, "claimTask", "", pending).await?;

                Ok(ClaimGrant {
                    claim: Claim {
                        id: claim_id,
                        task_id,
                        actor_id,
                        phase,
                        submission_ids: Vec::new(),
                        acquired_at: now,
                        expires_at,
                        status: ClaimStatus::Active,
                        task_revision: task_rev,
                        plan_revision_ids: plan_revision_id.into_iter().collect(),
                    },
                    lease_token,
                })
            })
        })
        .await
    }

    pub async fn renew_claim(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        claim_id: Uuid,
        lease_token: &str,
        input: RenewInput,
        request_hash: &str,
    ) -> Result<Replay<Claim>, DomainError> {
        let ttl = validate_ttl(input.ttl_seconds)?;
        let actor_id = ctx.actor.id;
        let now = ctx.now;
        let command_id = ctx.command_id;
        let token = lease_token.to_string();

        self.idempotent_transaction(&ctx, request_hash, 200, move |tx| {
            Box::pin(async move {
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
                if row.expires_at <= now {
                    return Err(DomainError::LeaseInvalid);
                }

                let new_expires = now + TimeDelta::seconds(ttl);
                let new_expires_text = format_ts(&new_expires);
                sqlx::query("UPDATE claims SET expires_at = ?1 WHERE id = ?2")
                    .bind(&new_expires_text)
                    .bind(claim_id.to_string())
                    .execute(&mut **tx)
                    .await?;

                let actor = super::live_actor(tx, &actor_id).await?;
                let ctx_for_events = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                append_events(
                    tx,
                    &project,
                    &ctx_for_events,
                    "renewClaim",
                    "",
                    vec![PendingEvent::claim(
                        claim_id,
                        row.task_revision,
                        row.task_id,
                    )],
                )
                .await?;

                Ok(Claim {
                    id: row.id,
                    task_id: row.task_id,
                    actor_id: row.actor_id,
                    phase: row.phase,
                    submission_ids: row.submission_id.into_iter().collect(),
                    acquired_at: row.acquired_at,
                    expires_at: new_expires,
                    status: ClaimStatus::Active,
                    task_revision: row.task_revision,
                    plan_revision_ids: row.plan_revision_id.into_iter().collect(),
                })
            })
        })
        .await
    }

    pub async fn release_claim(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        claim_id: Uuid,
        lease_token: &str,
        request_hash: &str,
    ) -> Result<Replay<Claim>, DomainError> {
        let actor_id = ctx.actor.id;
        let now = ctx.now;
        let command_id = ctx.command_id;
        let token = lease_token.to_string();

        self.idempotent_transaction(&ctx, request_hash, 200, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&now);

                let initial = load_claim_row(tx, &claim_id, &project).await?;
                let reconciled = reconcile_task_claims(tx, initial.task_id, &now_text).await?;

                // Re-load after reconciliation: the claim may have been expired.
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

                sqlx::query(
                    "UPDATE claims SET status = 'released', closed_at = ?1, \
                     close_reason = '' WHERE id = ?2",
                )
                .bind(&now_text)
                .bind(claim_id.to_string())
                .execute(&mut **tx)
                .await?;

                let actor = super::live_actor(tx, &actor_id).await?;
                let ctx_for_events = CommandContext {
                    actor,
                    command_id,
                    idempotency_key: Uuid::nil(),
                    expected_revision: None,
                    now,
                };
                let mut pending: Vec<PendingEvent> = reconciled
                    .iter()
                    .map(|c| PendingEvent::claim(c.id, c.task_revision, c.task_id))
                    .collect();
                pending.push(PendingEvent::claim(
                    claim_id,
                    row.task_revision,
                    row.task_id,
                ));
                append_events(tx, &project, &ctx_for_events, "releaseClaim", "", pending).await?;

                Ok(claim_from_row(&row, ClaimStatus::Released))
            })
        })
        .await
    }
}
