use chrono::TimeDelta;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use super::hierarchy::{filter_token, project_exists};
use super::{
    ListParams, Page, decode_rowid_cursor, effective_limit, encode_rowid_cursor, split_page,
};
use crate::error::DomainError;
use crate::model::{ActorId, CommandId, Event, EventId, EventType, ProjectId};
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

/// Replay page bound per connection establishment (plan/08).
pub const REPLAY_MAX_EVENTS: i64 = 1_000;
/// Replay accepts cursors from the last seven days only (plan/08).
pub const REPLAY_WINDOW_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncReason {
    CursorExpired,
    CursorAhead,
    BacklogLimit,
}

impl ResyncReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ResyncReason::CursorExpired => "cursor_expired",
            ResyncReason::CursorAhead => "cursor_ahead",
            ResyncReason::BacklogLimit => "backlog_limit",
        }
    }
}

/// Bounded replay result, independent from HTTP streaming (plan/08); step 015
/// maps `ResyncRequired` to the SSE `resync_required` frame.
#[derive(Debug)]
pub enum ReplayOutcome {
    Events(Vec<Event>),
    ResyncRequired {
        current_cursor: EventId,
        reason: ResyncReason,
    },
}

pub(crate) fn event_from_row(row: &SqliteRow) -> Result<Event, DomainError> {
    let project_id: String = row.try_get("project_id")?;
    let actor_id: String = row.try_get("actor_id")?;
    let command_id: String = row.try_get("command_id")?;
    let event_type: String = row.try_get("type")?;
    let resource_id: String = row.try_get("resource_id")?;
    let occurred_at: String = row.try_get("occurred_at")?;
    let affected_ids: String = row.try_get("affected_ids")?;
    let affected: Vec<String> = serde_json::from_str(&affected_ids)
        .map_err(|err| StorageError::Corrupt(format!("events.affected_ids: {err}")))?;
    Ok(Event {
        id: row.try_get("id")?,
        project_id: ProjectId::from_uuid(parse_uuid("events.project_id", &project_id)?),
        actor_id: ActorId::from_uuid(parse_uuid("events.actor_id", &actor_id)?),
        command_id: CommandId::from_uuid(parse_uuid("events.command_id", &command_id)?),
        event_type: EventType::parse(&event_type)
            .ok_or_else(|| StorageError::Corrupt(format!("events.type: {event_type}")))?,
        resource_id: parse_uuid("events.resource_id", &resource_id)?,
        resource_revision: row.try_get("resource_revision")?,
        occurred_at: parse_ts("events.occurred_at", &occurred_at)?,
        affected_ids: affected
            .iter()
            .map(|id| parse_uuid("events.affected_ids", id))
            .collect::<Result<Vec<_>, _>>()?,
        action: row.try_get("action")?,
        reason: row.try_get("reason")?,
    })
}

const EVENT_COLUMNS: &str = "id, project_id, actor_id, command_id, type, resource_id, \
     resource_revision, occurred_at, affected_ids, action, reason";

fn history_filter(project: &ProjectId, resource_id: Option<Uuid>) -> String {
    format!("project={project}&resource={}", filter_token(resource_id))
}

pub(crate) async fn list_history(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    resource_id: Option<Uuid>,
    params: &ListParams,
) -> Result<Page<Event>, DomainError> {
    let limit = effective_limit(params)?;
    // Route membership before cursor validity (plan/04).
    if !project_exists(conn, project).await? {
        return Err(DomainError::NotFound);
    }
    let filter = history_filter(project, resource_id);
    let after = params
        .cursor
        .as_deref()
        .map(|cursor| decode_rowid_cursor(cursor, "listHistory", &filter))
        .transpose()?;

    let mut builder = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {EVENT_COLUMNS} FROM events WHERE project_id = "
    ));
    builder.push_bind(project.to_string());
    if let Some(resource) = resource_id {
        builder
            .push(" AND resource_id = ")
            .push_bind(resource.to_string());
    }
    if let Some(cursor_id) = after {
        builder.push(" AND id < ").push_bind(cursor_id);
    }
    // History reads all retained rows, newest first (plan/08).
    builder
        .push(" ORDER BY id DESC LIMIT ")
        .push_bind(limit + 1);

    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(event_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(split_page(items, limit, |event: &Event| {
        encode_rowid_cursor("listHistory", &filter, event.id)
    }))
}

impl Store {
    pub async fn list_history(
        &self,
        project: &ProjectId,
        resource_id: Option<Uuid>,
        params: &ListParams,
    ) -> Result<Page<Event>, DomainError> {
        let mut tx = self.read_transaction().await?;
        let page = list_history(&mut tx, project, resource_id, params).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(page)
    }

    /// Global high-water mark; the SSE "no cursor" starting point in step 015.
    pub async fn event_high_water(&self) -> Result<EventId, DomainError> {
        let max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM events")
            .fetch_one(self.pool())
            .await
            .map_err(StorageError::from)?;
        Ok(max)
    }

    pub async fn replay_events(
        &self,
        project: &ProjectId,
        after: EventId,
    ) -> Result<ReplayOutcome, DomainError> {
        let now = self.clock().now();
        let mut tx = self.read_transaction().await?;
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        let global_max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM events")
            .fetch_one(&mut *tx)
            .await
            .map_err(StorageError::from)?;
        // Backup restore can roll the cursor ahead of the database (plan/08).
        if after > global_max {
            return Ok(ReplayOutcome::ResyncRequired {
                current_cursor: global_max,
                reason: ResyncReason::CursorAhead,
            });
        }
        // The first indexed row after the cursor is the oldest; one probe tells
        // whether the cursor predates the replay window.
        let horizon = format_ts(&(now - TimeDelta::days(REPLAY_WINDOW_DAYS)));
        let expired = sqlx::query(
            "SELECT 1 FROM events WHERE project_id = ?1 AND id > ?2 AND occurred_at < ?3 LIMIT 1",
        )
        .bind(project.to_string())
        .bind(after)
        .bind(&horizon)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?
        .is_some();
        if expired {
            return Ok(ReplayOutcome::ResyncRequired {
                current_cursor: global_max,
                reason: ResyncReason::CursorExpired,
            });
        }
        let backlog: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM \
             (SELECT 1 FROM events WHERE project_id = ?1 AND id > ?2 LIMIT ?3)",
        )
        .bind(project.to_string())
        .bind(after)
        .bind(REPLAY_MAX_EVENTS + 1)
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        if backlog > REPLAY_MAX_EVENTS {
            return Ok(ReplayOutcome::ResyncRequired {
                current_cursor: global_max,
                reason: ResyncReason::BacklogLimit,
            });
        }
        // AssertSqlSafe: interpolates a compile-time column list only.
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {EVENT_COLUMNS} FROM events WHERE project_id = ?1 AND id > ?2 \
             ORDER BY id ASC LIMIT ?3"
        )))
        .bind(project.to_string())
        .bind(after)
        .bind(REPLAY_MAX_EVENTS)
        .fetch_all(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        tx.commit().await.map_err(StorageError::from)?;
        let events = rows
            .iter()
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ReplayOutcome::Events(events))
    }
}
