use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqliteConnection};

use crate::commands::CommandId;
use crate::error::DomainError;
use crate::model::{Actor, ActorId, ActorKind};
use crate::storage::StorageError;
use crate::storage::rows::{format_ts, parse_flag, parse_ts, parse_uuid};

pub(super) async fn insert_credential(
    conn: &mut SqliteConnection,
    actor: &ActorId,
    token_hash: &str,
    now: &DateTime<Utc>,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO credentials (id, actor_id, token_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(crate::model::new_v7(*now).to_string())
    .bind(actor.to_string())
    .bind(token_hash)
    .bind(format_ts(now))
    .execute(conn)
    .await?;
    Ok(())
}

pub(super) async fn security_audit(
    conn: &mut SqliteConnection,
    actor: &ActorId,
    action: &str,
    target: Option<&ActorId>,
    reason: &str,
    command_id: &CommandId,
    now: &DateTime<Utc>,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO security_audit (actor_id, action, target_actor_id, reason, command_id, \
         created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(actor.to_string())
    .bind(action)
    .bind(target.map(ActorId::to_string))
    .bind(reason)
    .bind(command_id.to_string())
    .bind(format_ts(now))
    .execute(conn)
    .await?;
    Ok(())
}

pub(super) fn actor_from_joined_row(row: &SqliteRow) -> Result<Actor, DomainError> {
    let id: String = row.try_get("id").map_err(StorageError::from)?;
    let kind: String = row.try_get("kind").map_err(StorageError::from)?;
    let created_at: String = row.try_get("created_at").map_err(StorageError::from)?;
    Ok(Actor {
        id: ActorId::from_uuid(parse_uuid("actors.id", &id)?),
        kind: ActorKind::parse(&kind)
            .ok_or_else(|| StorageError::Corrupt(format!("actors.kind: {kind:?}")))?,
        label: row.try_get("label").map_err(StorageError::from)?,
        revoked: parse_flag(
            "actors.revoked",
            row.try_get("revoked").map_err(StorageError::from)?,
        )?,
        created_at: parse_ts("actors.created_at", &created_at)?,
    })
}

// The indexed equality on the SHA-256 digest is the credential check; both
// sides are hashes, so there is nothing left to compare in constant time.
pub(super) async fn credential_actor_by_hash<'e, E>(
    executor: E,
    digest: &str,
) -> Result<Option<Actor>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(row) = sqlx::query(
        "SELECT a.id, a.kind, a.label, a.revoked, a.created_at \
         FROM credentials c JOIN actors a ON a.id = c.actor_id \
         WHERE c.token_hash = ?1 AND c.revoked_at IS NULL",
    )
    .bind(digest)
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    Ok(Some(actor_from_joined_row(&row)?))
}

pub(super) async fn live_owner_credential<'e, E>(
    executor: E,
) -> Result<Option<(Actor, String)>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(row) = sqlx::query(
        "SELECT a.id, a.kind, a.label, a.revoked, a.created_at, c.token_hash \
         FROM credentials c JOIN actors a ON a.id = c.actor_id \
         WHERE a.kind = 'human' AND c.revoked_at IS NULL \
         ORDER BY c.created_at DESC, c.id DESC LIMIT 1",
    )
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    let hash: String = row.try_get("token_hash").map_err(StorageError::from)?;
    Ok(Some((actor_from_joined_row(&row)?, hash)))
}

// Reissue also recovers an owner whose credentials were all revoked.
pub(super) async fn live_owner_credential_or_actor<'e, E>(
    executor: E,
) -> Result<Option<(Actor, Option<String>)>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite> + Copy,
{
    if let Some((actor, hash)) = live_owner_credential(executor).await? {
        return Ok(Some((actor, Some(hash))));
    }
    let Some(row) = sqlx::query(
        "SELECT id, kind, label, revoked, created_at FROM actors \
         WHERE kind = 'human' ORDER BY created_at ASC, id ASC LIMIT 1",
    )
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    Ok(Some((actor_from_joined_row(&row)?, None)))
}
