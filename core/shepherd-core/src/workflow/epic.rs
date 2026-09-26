use sqlx::SqliteConnection;

use crate::commands::{
    ClaimScope, CommandContext, PendingEvent, revoke_active_claims, withdraw_pending_submissions,
};
use crate::error::DomainError;
use crate::model::{Counts, Epic, EpicStatus, Revision, TaskId, TaskPhase, TaskStatus};
use crate::storage::rows::{format_ts, parse_uuid};

// Auto-completion needs at least one non-waived task and every one of them done;
// the empty/all-waived shape is reserved for the owner's explicit complete (plan/04).
pub(crate) fn auto_completable(counts: Counts) -> bool {
    let required = counts.total - counts.waived;
    required >= 1 && counts.done == required
}

pub(crate) fn explicitly_completable(counts: Counts) -> bool {
    counts.total == 0 || counts.waived == counts.total
}

pub(crate) async fn block(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    epic: &Epic,
    reason: &str,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = epic.revision.next();
    let now = format_ts(&ctx.now);
    sqlx::query(
        "UPDATE epics SET revision = ?1, updated_at = ?2, block_actor_id = ?3, \
         block_reason = ?4, block_created_at = ?2 WHERE id = ?5",
    )
    .bind(next.value())
    .bind(&now)
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(epic.id.to_string())
    .execute(&mut *conn)
    .await?;
    // Every descendant claim goes, planning and review included; pending
    // submissions stay pending (plan/04). Each claim-revoked task changed
    // representation, so it gets its once-per-command bump and event.
    let revoked = revoke_active_claims(conn, ClaimScope::Epic(epic.id), &now, reason).await?;
    for claim in &revoked {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    let tasks: Vec<TaskId> = revoked.iter().map(|claim| claim.task_id).collect();
    for (task, revision) in bump_tasks(conn, &tasks, &now).await? {
        events.push(PendingEvent::task(task, revision, epic.id));
    }
    events.push(PendingEvent::epic(epic.id, next, epic.goal_id));
    Ok(next)
}

pub(crate) async fn unblock(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    epic: &Epic,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = epic.revision.next();
    sqlx::query(
        "UPDATE epics SET revision = ?1, updated_at = ?2, block_actor_id = NULL, \
         block_reason = NULL, block_created_at = NULL WHERE id = ?3",
    )
    .bind(next.value())
    .bind(format_ts(&ctx.now))
    .bind(epic.id.to_string())
    .execute(&mut *conn)
    .await?;
    events.push(PendingEvent::epic(epic.id, next, epic.goal_id));
    Ok(next)
}

pub(crate) async fn cancel(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    epic: &Epic,
    reason: &str,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = epic.revision.next();
    let now = format_ts(&ctx.now);
    for claim in revoke_active_claims(conn, ClaimScope::Epic(epic.id), &now, reason).await? {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    for submission in
        withdraw_pending_submissions(conn, ClaimScope::Epic(epic.id), &now, reason).await?
    {
        events.push(PendingEvent::submission(
            submission.id,
            submission.revision,
            submission.task_id,
        ));
    }
    // Done descendants are preserved; every nonterminal task cancels with the
    // propagated reason (plan/04).
    let cancelled: Vec<(String, i64)> = sqlx::query_as(
        "UPDATE tasks SET revision = revision + 1, updated_at = ?1, status = ?2, phase = ?3, \
         cancellation_actor_id = ?4, cancellation_reason = ?5, cancellation_created_at = ?1 \
         WHERE epic_id = ?6 AND status NOT IN ('done', 'cancelled') RETURNING id, revision",
    )
    .bind(&now)
    .bind(TaskStatus::Cancelled.as_str())
    .bind(TaskPhase::Complete.as_str())
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(epic.id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    for (id, revision) in cancelled {
        let task = TaskId::from_uuid(parse_uuid("tasks.id", &id)?);
        let revision = Revision::from_stored(revision)
            .ok_or_else(|| crate::storage::StorageError::Corrupt("task revision".into()))?;
        events.push(PendingEvent::task(task, revision, epic.id));
    }
    sqlx::query(
        "UPDATE epics SET revision = ?1, updated_at = ?2, status = ?3, \
         cancellation_actor_id = ?4, cancellation_reason = ?5, cancellation_created_at = ?2 \
         WHERE id = ?6",
    )
    .bind(next.value())
    .bind(&now)
    .bind(EpicStatus::Cancelled.as_str())
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(epic.id.to_string())
    .execute(&mut *conn)
    .await?;
    events.push(PendingEvent::epic(epic.id, next, epic.goal_id));
    Ok(next)
}

pub(crate) async fn complete(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    epic: &Epic,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = epic.revision.next();
    sqlx::query("UPDATE epics SET revision = ?1, updated_at = ?2, status = ?3 WHERE id = ?4")
        .bind(next.value())
        .bind(format_ts(&ctx.now))
        .bind(EpicStatus::Done.as_str())
        .bind(epic.id.to_string())
        .execute(&mut *conn)
        .await?;
    events.push(PendingEvent::epic(epic.id, next, epic.goal_id));
    Ok(next)
}

async fn bump_tasks(
    conn: &mut SqliteConnection,
    tasks: &[TaskId],
    now: &str,
) -> Result<Vec<(TaskId, Revision)>, DomainError> {
    let mut bumped = Vec::with_capacity(tasks.len());
    for task in tasks {
        let revision: i64 = sqlx::query_scalar(
            "UPDATE tasks SET revision = revision + 1, updated_at = ?1 WHERE id = ?2 \
             RETURNING revision",
        )
        .bind(now)
        .bind(task.to_string())
        .fetch_one(&mut *conn)
        .await?;
        let revision = Revision::from_stored(revision)
            .ok_or_else(|| crate::storage::StorageError::Corrupt("task revision".into()))?;
        bumped.push((*task, revision));
    }
    Ok(bumped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(total: i64, done: i64, cancelled: i64, waived: i64) -> Counts {
        Counts {
            total,
            done,
            cancelled,
            waived,
        }
    }

    #[test]
    fn auto_completion_needs_every_nonwaived_task_done() {
        assert!(auto_completable(counts(3, 3, 0, 0)));
        assert!(auto_completable(counts(3, 2, 1, 1)));
        assert!(!auto_completable(counts(3, 2, 0, 0)));
        // A cancelled task without a waiver keeps blocking completion.
        assert!(!auto_completable(counts(3, 2, 1, 0)));
        // Empty and all-waived epics are the owner's explicit call.
        assert!(!auto_completable(counts(0, 0, 0, 0)));
        assert!(!auto_completable(counts(2, 0, 2, 2)));
    }

    #[test]
    fn explicit_completion_covers_empty_and_all_waived_only() {
        assert!(explicitly_completable(counts(0, 0, 0, 0)));
        assert!(explicitly_completable(counts(2, 0, 2, 2)));
        assert!(!explicitly_completable(counts(3, 3, 0, 0)));
        assert!(!explicitly_completable(counts(3, 2, 1, 1)));
    }
}
