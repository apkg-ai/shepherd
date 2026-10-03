use sqlx::SqliteConnection;

use crate::commands::{
    ClaimScope, CommandContext, PendingEvent, revoke_active_claims, withdraw_pending_submissions,
};
use crate::error::DomainError;
use crate::model::{Revision, Task, TaskPhase, TaskStatus};
use crate::storage::rows::format_ts;

// Write transitions from plan/04; every guard lives in the calling command.
// Each bumps the task exactly once and pushes the events it is responsible for.

pub(crate) async fn block(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    task: &Task,
    reason: &str,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = task.revision.next();
    let now = format_ts(&ctx.now);
    sqlx::query(
        "UPDATE tasks SET revision = ?1, updated_at = ?2, block_actor_id = ?3, \
         block_reason = ?4, block_created_at = ?2 WHERE id = ?5",
    )
    .bind(next.value())
    .bind(&now)
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(task.id.to_string())
    .execute(&mut *conn)
    .await?;
    // Pending submissions stay pending on block (plan/04).
    let closed = revoke_active_claims(conn, ClaimScope::Task(task.id), &now, reason).await?;
    for claim in closed.all() {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    events.push(PendingEvent::task(task.id, next, task.epic_id));
    Ok(next)
}

pub(crate) async fn unblock(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    task: &Task,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = task.revision.next();
    sqlx::query(
        "UPDATE tasks SET revision = ?1, updated_at = ?2, block_actor_id = NULL, \
         block_reason = NULL, block_created_at = NULL WHERE id = ?3",
    )
    .bind(next.value())
    .bind(format_ts(&ctx.now))
    .bind(task.id.to_string())
    .execute(&mut *conn)
    .await?;
    events.push(PendingEvent::task(task.id, next, task.epic_id));
    Ok(next)
}

pub(crate) async fn cancel(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    task: &Task,
    reason: &str,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = task.revision.next();
    let now = format_ts(&ctx.now);
    sqlx::query(
        "UPDATE tasks SET revision = ?1, updated_at = ?2, status = ?3, phase = ?4, \
         cancellation_actor_id = ?5, cancellation_reason = ?6, cancellation_created_at = ?2 \
         WHERE id = ?7",
    )
    .bind(next.value())
    .bind(&now)
    .bind(TaskStatus::Cancelled.as_str())
    .bind(TaskPhase::Complete.as_str())
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(task.id.to_string())
    .execute(&mut *conn)
    .await?;
    let closed = revoke_active_claims(conn, ClaimScope::Task(task.id), &now, reason).await?;
    for claim in closed.all() {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    for submission in
        withdraw_pending_submissions(conn, ClaimScope::Task(task.id), &now, reason).await?
    {
        events.push(PendingEvent::submission(
            submission.id,
            submission.revision,
            submission.task_id,
        ));
    }
    events.push(PendingEvent::task(task.id, next, task.epic_id));
    Ok(next)
}

pub(crate) async fn waive(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    task: &Task,
    reason: &str,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = task.revision.next();
    sqlx::query(
        "UPDATE tasks SET revision = ?1, updated_at = ?2, waiver_actor_id = ?3, \
         waiver_reason = ?4, waiver_created_at = ?2 WHERE id = ?5",
    )
    .bind(next.value())
    .bind(format_ts(&ctx.now))
    .bind(ctx.actor.id.to_string())
    .bind(reason)
    .bind(task.id.to_string())
    .execute(&mut *conn)
    .await?;
    events.push(PendingEvent::task(task.id, next, task.epic_id));
    Ok(next)
}

// The done transition; step 010's successful execute report reuses this path
// (hence the test-only consumers today).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn complete(
    conn: &mut SqliteConnection,
    ctx: &CommandContext,
    task: &Task,
    events: &mut Vec<PendingEvent>,
) -> Result<Revision, DomainError> {
    let next = task.revision.next();
    sqlx::query(
        "UPDATE tasks SET revision = ?1, updated_at = ?2, status = ?3, phase = ?4 WHERE id = ?5",
    )
    .bind(next.value())
    .bind(format_ts(&ctx.now))
    .bind(TaskStatus::Done.as_str())
    .bind(TaskPhase::Complete.as_str())
    .bind(task.id.to_string())
    .execute(&mut *conn)
    .await?;
    events.push(PendingEvent::task(task.id, next, task.epic_id));
    Ok(next)
}
