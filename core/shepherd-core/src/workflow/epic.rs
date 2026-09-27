use std::collections::BTreeSet;

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
    let closed = revoke_active_claims(conn, ClaimScope::Epic(epic.id), &now, reason).await?;
    for claim in closed.all() {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    let tasks: Vec<TaskId> = closed.revoked.iter().map(|claim| claim.task_id).collect();
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
    let closed = revoke_active_claims(conn, ClaimScope::Epic(epic.id), &now, reason).await?;
    for claim in closed.all() {
        events.push(PendingEvent::claim(
            claim.id,
            claim.task_revision,
            claim.task_id,
        ));
    }
    let mut settled: BTreeSet<TaskId> = closed.revoked.iter().map(|claim| claim.task_id).collect();
    for submission in
        withdraw_pending_submissions(conn, ClaimScope::Epic(epic.id), &now, reason).await?
    {
        settled.insert(submission.task_id);
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
        settled.remove(&task);
        let revision = Revision::from_stored(revision)
            .ok_or_else(|| crate::storage::StorageError::Corrupt("task revision".into()))?;
        events.push(PendingEvent::task(task, revision, epic.id));
    }
    // A preserved done task whose claim was revoked or submission withdrawn
    // changed representation too: same once-per-command bump as epic::block.
    let settled: Vec<TaskId> = settled.into_iter().collect();
    for (task, revision) in bump_tasks(conn, &settled, &now).await? {
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

// SQLite caps bound parameters per statement (999 before 3.32, 32,766 after);
// chunked IN lists keep blockEpic safe at any task count.
const TASK_CHUNK: usize = 500;

async fn bump_tasks(
    conn: &mut SqliteConnection,
    tasks: &[TaskId],
    now: &str,
) -> Result<Vec<(TaskId, Revision)>, DomainError> {
    let mut bumped = Vec::new();
    for chunk in tasks.chunks(TASK_CHUNK) {
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "UPDATE tasks SET revision = revision + 1, updated_at = ",
        );
        builder.push_bind(now).push(" WHERE id IN (");
        let mut separated = builder.separated(", ");
        for task in chunk {
            separated.push_bind(task.to_string());
        }
        builder.push(") RETURNING id, revision");
        let rows: Vec<(String, i64)> = builder
            .build_query_as()
            .fetch_all(&mut *conn)
            .await
            .map_err(crate::storage::StorageError::from)?;
        let parsed: Result<Vec<(TaskId, Revision)>, DomainError> = rows
            .into_iter()
            .map(|(id, revision)| {
                let task = TaskId::from_uuid(parse_uuid("tasks.id", &id)?);
                let revision = Revision::from_stored(revision)
                    .ok_or_else(|| crate::storage::StorageError::Corrupt("task revision".into()))?;
                Ok((task, revision))
            })
            .collect();
        bumped.extend(parsed?);
    }
    Ok(bumped)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::model::{Actor, ActorKind, Clock, EpicCreate, GoalCreate, ProjectCreate, TestClock};
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};
    use uuid::Uuid;

    fn ctx(actor: &Actor, clock: &TestClock) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: crate::model::CommandId::generate(clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision: None,
            now: clock.now(),
        }
    }

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

    // 501 revoked-claim tasks = one full chunk plus the boundary remainder.
    #[tokio::test]
    async fn bump_tasks_crosses_the_chunk_boundary_without_lost_or_double_bumps() {
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
        let seeded = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &seeded).await }))
            .await
            .unwrap();
        let project = store
            .create_project(
                ctx(&owner, &clock),
                ProjectCreate {
                    name: "P".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        let goal = store
            .create_goal(
                ctx(&owner, &clock),
                project.id,
                GoalCreate {
                    title: "G".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;
        let epic = store
            .create_epic(
                ctx(&owner, &clock),
                project.id,
                goal.id,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;

        let ids: Vec<TaskId> = (0..501).map(|_| TaskId::generate(clock.now())).collect();
        let now = format_ts(&clock.now());
        let mut tx = store.pool().begin().await.unwrap();
        for (i, id) in ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, \
                 epic_id, title, description, type_key, status, phase, planning_required, \
                 plan_review, work_review, archived, attempt_count) \
                 VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5, '', 'code', 'open', 'execution', 0, \
                 'none', 'none', 0, 0)",
            )
            .bind(id.to_string())
            .bind(&now)
            .bind(project.id.to_string())
            .bind(epic.id.to_string())
            .bind(format!("task-{i}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let mut conn = store.pool().acquire().await.unwrap();
        let bumped = bump_tasks(&mut conn, &ids, &now).await.unwrap();

        assert_eq!(bumped.len(), 501);
        assert_eq!(
            bumped.iter().map(|(id, _)| *id).collect::<HashSet<_>>(),
            ids.iter().copied().collect::<HashSet<_>>()
        );
        assert!(bumped.iter().all(|(_, revision)| revision.value() == 2));
    }
}
