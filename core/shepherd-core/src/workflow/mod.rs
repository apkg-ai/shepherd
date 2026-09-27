pub mod eligibility;
pub(crate) mod epic;
pub mod policy;
pub(crate) mod task;

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, Sqlite, SqliteConnection};

use crate::commands::PendingEvent;
use crate::error::DomainError;
use crate::model::{EpicId, EpicStatus, ProjectId};
use crate::storage::rows::parse_uuid;

pub(crate) struct AffectedScope {
    pub(crate) project: ProjectId,
    pub(crate) epics: Vec<EpicId>,
}

// Synchronous completion cascade (plan/04): recompute epic completion from the
// affected epics and follow dependents of every completion until no state
// changes remain. Stops at proposed, blocked, cancelled, done and archived
// epics. The scoped dependency graph is acyclic (guarded at insert), and
// completion is monotone, so the worklist terminates.
//
// Contract: a caller that bumps an epic row in the same command MUST push its
// epic.changed onto `events` before calling recompute — the pending-event scan
// below is the once-per-command bump mechanism (plan/08: every bump emits an
// event), and the reuse branch relies on that earlier write for revision and
// updated_at.
pub(crate) async fn recompute(
    conn: &mut SqliteConnection,
    scope: AffectedScope,
    events: &mut Vec<PendingEvent>,
    now: DateTime<Utc>,
) -> Result<(), DomainError> {
    let mut frontier: BTreeSet<EpicId> = scope.epics.into_iter().collect();
    while !frontier.is_empty() {
        let wave: Vec<EpicId> = frontier.iter().copied().collect();
        frontier.clear();
        let snapshots = eligibility::load_epic_snapshots(conn, &scope.project, &wave, now).await?;
        let mut completed = Vec::new();
        for id in wave {
            let Some(snapshot) = snapshots.get(&id) else {
                continue;
            };
            let current = &snapshot.epic;
            if !matches!(current.status, EpicStatus::Open | EpicStatus::Active)
                || current.block.is_some()
                || current.archived
                || snapshot.goal_archived
                || snapshot.project_archived
                || !snapshot
                    .epic_prereqs
                    .iter()
                    .all(|(_, status)| *status == EpicStatus::Done)
                || !epic::auto_completable(snapshot.task_counts)
            {
                continue;
            }
            // A command bumps each resource once: reuse the revision of an
            // epic event already pending from this command's earlier writes.
            let pending = events.iter().any(|event| {
                event.event_type == "epic.changed" && event.resource_id == id.as_uuid()
            });
            if pending {
                sqlx::query("UPDATE epics SET status = ?1 WHERE id = ?2")
                    .bind(EpicStatus::Done.as_str())
                    .bind(id.to_string())
                    .execute(&mut *conn)
                    .await?;
            } else {
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE epics SET revision = ?1, updated_at = ?2, status = ?3 WHERE id = ?4",
                )
                .bind(next.value())
                .bind(crate::storage::rows::format_ts(&now))
                .bind(EpicStatus::Done.as_str())
                .bind(id.to_string())
                .execute(&mut *conn)
                .await?;
                events.push(PendingEvent::epic(id, next, current.goal_id));
            }
            completed.push(id);
        }
        if completed.is_empty() {
            break;
        }
        frontier.extend(dependents_of(conn, &completed).await?);
    }
    Ok(())
}

pub(crate) async fn dependents_of(
    conn: &mut SqliteConnection,
    prerequisites: &[EpicId],
) -> Result<Vec<EpicId>, DomainError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT dependent_id FROM epic_dependencies WHERE prerequisite_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in prerequisites {
        separated.push_bind(id.to_string());
    }
    builder.push(")");
    let rows: Vec<String> = builder
        .build_query_scalar()
        .fetch_all(&mut *conn)
        .await
        .map_err(crate::storage::StorageError::from)?;
    rows.iter()
        .map(|id| {
            Ok(EpicId::from_uuid(parse_uuid(
                "epic_dependencies.dependent_id",
                id,
            )?))
        })
        .collect()
}
