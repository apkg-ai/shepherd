mod dependencies;
mod hierarchy;

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqliteConnection, Transaction};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Actor, ActorId, ActorKind, CommandId, DependencyId, EpicId, EventId, GoalId, ProjectId,
    Revision, TaskId, TaskTypeId,
};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store, rows};

pub struct CommandContext {
    pub actor: Actor,
    pub command_id: CommandId,
    /// Carried per plan/05; replay lookup lands with the real codec in step 007.
    pub idempotency_key: Uuid,
    pub expected_revision: Option<i64>,
    pub now: DateTime<Utc>,
}

#[derive(Debug)]
pub struct CommandResult<T> {
    pub value: T,
    pub events: Vec<EventId>,
}

pub(crate) type DomainTxFuture<'t, T> =
    Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 't>>;

impl Store {
    pub(crate) async fn domain_transaction<T, F>(&self, command: F) -> Result<T, DomainError>
    where
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> DomainTxFuture<'t, T>,
    {
        let mut tx = self.begin_command().await?;
        match command(&mut tx).await {
            Ok(value) => {
                tx.commit().await.map_err(StorageError::from)?;
                Ok(value)
            }
            Err(err) => {
                tx.rollback().await.ok();
                Err(err)
            }
        }
    }
}

// Authenticated callers are rechecked inside the transaction (plan/05).
pub(crate) async fn live_actor(
    conn: &mut SqliteConnection,
    id: &ActorId,
) -> Result<Actor, DomainError> {
    match rows::get_actor(&mut *conn, id).await? {
        Some(actor) if !actor.revoked => Ok(actor),
        _ => Err(DomainError::Forbidden(
            "actor is not registered or is revoked".into(),
        )),
    }
}

pub(crate) fn require_owner(actor: &Actor) -> Result<(), DomainError> {
    match actor.kind {
        ActorKind::Human => Ok(()),
        ActorKind::Agent => Err(DomainError::Forbidden("owner capability required".into())),
    }
}

pub(crate) fn require_revision(expected: Option<i64>, actual: Revision) -> Result<(), DomainError> {
    match expected {
        None => Err(DomainError::PreconditionRequired),
        Some(value) if value == actual.value() => Ok(()),
        Some(value) => Err(DomainError::RevisionConflict {
            expected: value,
            actual: actual.value(),
        }),
    }
}

pub(crate) struct PendingEvent {
    event_type: &'static str,
    // Ownership order (project, goal, epic, task, registry) for deterministic event rows.
    kind_rank: u8,
    resource_id: Uuid,
    resource_revision: i64,
    affected_ids: Vec<Uuid>,
}

impl PendingEvent {
    pub(crate) fn project(id: ProjectId, revision: Revision) -> Self {
        Self {
            event_type: "project.changed",
            kind_rank: 0,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    pub(crate) fn goal(id: GoalId, revision: Revision) -> Self {
        Self {
            event_type: "goal.changed",
            kind_rank: 1,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    pub(crate) fn epic(id: EpicId, revision: Revision) -> Self {
        Self {
            event_type: "epic.changed",
            kind_rank: 2,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    pub(crate) fn task(id: TaskId, revision: Revision) -> Self {
        Self {
            event_type: "task.changed",
            kind_rank: 3,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    pub(crate) fn task_type(id: TaskTypeId, revision: Revision) -> Self {
        Self {
            event_type: "task_type.changed",
            kind_rank: 4,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: Vec::new(),
        }
    }

    // affected carries [dependent, prerequisite, scope] (plan/08: both endpoints
    // and affected scope).
    pub(crate) fn dependency(id: DependencyId, revision: Revision, affected: Vec<Uuid>) -> Self {
        Self {
            event_type: "dependency.changed",
            kind_rank: 5,
            resource_id: id.as_uuid(),
            resource_revision: revision.value(),
            affected_ids: affected,
        }
    }
}

pub(crate) async fn append_events(
    conn: &mut SqliteConnection,
    project_id: &ProjectId,
    ctx: &CommandContext,
    action: &'static str,
    mut events: Vec<PendingEvent>,
) -> Result<Vec<EventId>, DomainError> {
    // Deterministic order within a command: resource kind, then UUID (plan/08).
    events.sort_by_key(|event| (event.kind_rank, event.resource_id));
    let occurred_at = format_ts(&ctx.now);
    let mut ids = Vec::with_capacity(events.len());
    for event in &events {
        let affected: Vec<String> = event.affected_ids.iter().map(Uuid::to_string).collect();
        let affected_json = serde_json::to_string(&affected)
            .map_err(|err| StorageError::Corrupt(err.to_string()))?;
        let inserted = sqlx::query(
            "INSERT INTO events (action, reason, project_id, actor_id, command_id, type, \
             resource_id, resource_revision, occurred_at, affected_ids) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(action)
        .bind("")
        .bind(project_id.to_string())
        .bind(ctx.actor.id.to_string())
        .bind(ctx.command_id.to_string())
        .bind(event.event_type)
        .bind(event.resource_id.to_string())
        .bind(event.resource_revision)
        .bind(&occurred_at)
        .bind(&affected_json)
        .execute(&mut *conn)
        .await?;
        ids.push(inserted.last_insert_rowid());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(kind: ActorKind, revoked: bool) -> Actor {
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        Actor {
            id: ActorId::generate(now),
            kind,
            label: "someone".to_string(),
            revoked,
            created_at: now,
        }
    }

    #[test]
    fn only_humans_hold_the_owner_capability() {
        assert!(require_owner(&actor(ActorKind::Human, false)).is_ok());
        assert!(matches!(
            require_owner(&actor(ActorKind::Agent, false)),
            Err(DomainError::Forbidden(_))
        ));
    }

    #[test]
    fn revision_checks_follow_the_precondition_contract() {
        assert!(require_revision(Some(1), Revision::INITIAL).is_ok());
        assert!(matches!(
            require_revision(None, Revision::INITIAL),
            Err(DomainError::PreconditionRequired)
        ));
        assert!(matches!(
            require_revision(Some(2), Revision::INITIAL),
            Err(DomainError::RevisionConflict {
                expected: 2,
                actual: 1
            })
        ));
    }

    #[test]
    fn pending_events_sort_by_kind_then_uuid() {
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        let project = ProjectId::generate(now);
        let first_type = TaskTypeId::generate(now);
        let second_type = TaskTypeId::generate(now);
        let mut events = [
            PendingEvent::task_type(second_type, Revision::INITIAL),
            PendingEvent::task_type(first_type, Revision::INITIAL),
            PendingEvent::project(project, Revision::INITIAL),
        ];
        events.sort_by_key(|event| (event.kind_rank, event.resource_id));
        assert_eq!(events[0].event_type, "project.changed");
        assert_eq!(events[1].resource_id, first_type.as_uuid());
        assert_eq!(events[2].resource_id, second_type.as_uuid());
    }
}
