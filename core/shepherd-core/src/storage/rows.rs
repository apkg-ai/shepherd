use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Executor, Row, Sqlite};
use uuid::Uuid;

use super::{SealedResponse, StorageError};
use crate::model::{Actor, ActorId, ActorKind, CommandId};

pub fn format_ts(value: &DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_ts(column: &str, value: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|err| StorageError::Corrupt(format!("{column}: {value:?}: {err}")))
}

pub fn parse_uuid(column: &str, value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value)
        .map_err(|err| StorageError::Corrupt(format!("{column}: {value:?}: {err}")))
}

pub fn parse_flag(column: &str, value: i64) -> Result<bool, StorageError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(StorageError::Corrupt(format!("{column}: {other}"))),
    }
}

pub struct SchemaMetaRow {
    pub version: i64,
    pub export_version: String,
}

pub async fn read_schema_meta<'e, E>(executor: E) -> Result<SchemaMetaRow, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query("SELECT version, export_version FROM schema_meta")
        .fetch_one(executor)
        .await?;
    Ok(SchemaMetaRow {
        version: row.try_get("version")?,
        export_version: row.try_get("export_version")?,
    })
}

pub async fn insert_actor<'e, E>(executor: E, actor: &Actor) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO actors (id, kind, label, revoked, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(actor.id.to_string())
    .bind(actor.kind.as_str())
    .bind(&actor.label)
    .bind(i64::from(actor.revoked))
    .bind(format_ts(&actor.created_at))
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_actor<'e, E>(executor: E, id: &ActorId) -> Result<Option<Actor>, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("SELECT id, kind, label, revoked, created_at FROM actors WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(executor)
        .await?
        .map(actor_from_row)
        .transpose()
}

fn actor_from_row(row: SqliteRow) -> Result<Actor, StorageError> {
    let id: String = row.try_get("id")?;
    let kind: String = row.try_get("kind")?;
    let created_at: String = row.try_get("created_at")?;
    Ok(Actor {
        id: ActorId::from_uuid(parse_uuid("actors.id", &id)?),
        kind: ActorKind::parse(&kind)
            .ok_or_else(|| StorageError::Corrupt(format!("actors.kind: {kind:?}")))?,
        label: row.try_get("label")?,
        revoked: parse_flag("actors.revoked", row.try_get("revoked")?)?,
        created_at: parse_ts("actors.created_at", &created_at)?,
    })
}

pub struct IdempotencyRecord {
    pub actor_id: ActorId,
    pub key: Uuid,
    pub command_id: CommandId,
    pub request_hash: String,
    pub response_status: i64,
    pub sealed: SealedResponse,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub async fn put_idempotency<'e, E>(
    executor: E,
    record: &IdempotencyRecord,
) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO idempotency (actor_id, key, command_id, request_hash, response_status, \
         response_ciphertext, nonce, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(record.actor_id.to_string())
    .bind(record.key.to_string())
    .bind(record.command_id.to_string())
    .bind(&record.request_hash)
    .bind(record.response_status)
    .bind(&record.sealed.ciphertext)
    .bind(&record.sealed.nonce)
    .bind(format_ts(&record.created_at))
    .bind(format_ts(&record.expires_at))
    .execute(executor)
    .await?;
    Ok(())
}

// Fixed-width RFC 3339 keeps the TEXT comparison correct; expired rows are simply not returned.
pub async fn get_idempotency<'e, E>(
    executor: E,
    actor_id: &ActorId,
    key: &Uuid,
    now: DateTime<Utc>,
) -> Result<Option<IdempotencyRecord>, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "SELECT actor_id, key, command_id, request_hash, response_status, \
         response_ciphertext, nonce, created_at, expires_at \
         FROM idempotency WHERE actor_id = ?1 AND key = ?2 AND expires_at > ?3",
    )
    .bind(actor_id.to_string())
    .bind(key.to_string())
    .bind(format_ts(&now))
    .fetch_optional(executor)
    .await?
    .map(idempotency_from_row)
    .transpose()
}

fn idempotency_from_row(row: SqliteRow) -> Result<IdempotencyRecord, StorageError> {
    let actor_id: String = row.try_get("actor_id")?;
    let key: String = row.try_get("key")?;
    let command_id: String = row.try_get("command_id")?;
    let created_at: String = row.try_get("created_at")?;
    let expires_at: String = row.try_get("expires_at")?;
    Ok(IdempotencyRecord {
        actor_id: ActorId::from_uuid(parse_uuid("idempotency.actor_id", &actor_id)?),
        key: parse_uuid("idempotency.key", &key)?,
        command_id: CommandId::from_uuid(parse_uuid("idempotency.command_id", &command_id)?),
        request_hash: row.try_get("request_hash")?,
        response_status: row.try_get("response_status")?,
        sealed: SealedResponse {
            ciphertext: row.try_get("response_ciphertext")?,
            nonce: row.try_get("nonce")?,
        },
        created_at: parse_ts("idempotency.created_at", &created_at)?,
        expires_at: parse_ts("idempotency.expires_at", &expires_at)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_format_fixed_width_and_round_trip() {
        let now: DateTime<Utc> = "2026-09-14T00:00:01.020Z".parse().unwrap();
        let text = format_ts(&now);
        assert_eq!(text, "2026-09-14T00:00:01.020Z");
        assert_eq!(text.len(), 24);
        assert_eq!(parse_ts("created_at", &text).unwrap(), now);
    }

    #[test]
    fn fixed_width_timestamps_sort_lexicographically() {
        let earlier = format_ts(&"2026-09-14T00:00:09.999Z".parse().unwrap());
        let later = format_ts(&"2026-09-14T00:00:10.000Z".parse().unwrap());
        assert!(earlier < later);
    }

    #[test]
    fn malformed_stored_values_are_corrupt_errors() {
        assert!(matches!(
            parse_ts("created_at", "2026-02-30T00:00:00Z"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_ts("created_at", "not-a-time"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_uuid("id", "not-a-uuid"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_flag("revoked", 2),
            Err(StorageError::Corrupt(_))
        ));
    }

    #[test]
    fn flags_parse_stored_integers() {
        assert!(!parse_flag("revoked", 0).unwrap());
        assert!(parse_flag("revoked", 1).unwrap());
    }

    #[test]
    fn uuids_round_trip_through_text() {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        assert_eq!(parse_uuid("id", &id.to_string()).unwrap(), id);
    }

    use std::sync::Arc;

    use crate::model::{Clock, TestClock};
    use crate::storage::{Store, StoreOptions, TestCodec, TestKeyProvider, open};

    async fn test_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
        open(StoreOptions {
            db_path: dir.path().join("shepherd.db"),
            mvp_db_path: Some(dir.path().join("mvp").join("shepherd.db")),
            clock,
            codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([0; 32])))),
        })
        .await
        .unwrap()
    }

    fn test_actor(now: DateTime<Utc>) -> Actor {
        Actor {
            id: ActorId::generate(now),
            kind: ActorKind::Human,
            label: "owner".to_string(),
            revoked: false,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn actors_round_trip_through_their_table() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap()));
        let store = test_store(&dir, clock.clone()).await;
        let actor = test_actor(clock.now());
        insert_actor(store.pool(), &actor).await.unwrap();
        assert_eq!(
            get_actor(store.pool(), &actor.id).await.unwrap(),
            Some(actor.clone())
        );
        let missing = ActorId::generate(clock.now());
        assert_eq!(get_actor(store.pool(), &missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn idempotency_rows_round_trip_and_expire_with_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap()));
        let store = test_store(&dir, clock.clone()).await;
        let actor = test_actor(clock.now());
        insert_actor(store.pool(), &actor).await.unwrap();

        let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let record = IdempotencyRecord {
            actor_id: actor.id,
            key,
            command_id: CommandId::generate(clock.now()),
            request_hash: "hash-1".to_string(),
            response_status: 201,
            sealed: SealedResponse {
                ciphertext: vec![1, 2, 3],
                nonce: vec![9; 12],
            },
            created_at: clock.now(),
            expires_at: clock.now() + chrono::TimeDelta::days(7),
        };
        put_idempotency(store.pool(), &record).await.unwrap();

        let found = get_idempotency(store.pool(), &actor.id, &key, clock.now())
            .await
            .unwrap()
            .expect("record within TTL");
        assert_eq!(found.command_id, record.command_id);
        assert_eq!(found.request_hash, "hash-1");
        assert_eq!(found.response_status, 201);
        assert_eq!(found.sealed.ciphertext, vec![1, 2, 3]);
        assert_eq!(found.sealed.nonce, vec![9; 12]);
        assert_eq!(found.expires_at, record.expires_at);

        clock.advance(chrono::TimeDelta::days(7));
        assert!(
            get_idempotency(store.pool(), &actor.id, &key, clock.now())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn idempotency_rows_require_a_registered_actor() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap()));
        let store = test_store(&dir, clock.clone()).await;
        let record = IdempotencyRecord {
            actor_id: ActorId::generate(clock.now()),
            key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            command_id: CommandId::generate(clock.now()),
            request_hash: "hash-1".to_string(),
            response_status: 200,
            sealed: SealedResponse {
                ciphertext: vec![],
                nonce: vec![0; 12],
            },
            created_at: clock.now(),
            expires_at: clock.now() + chrono::TimeDelta::days(7),
        };
        let err = put_idempotency(store.pool(), &record).await.unwrap_err();
        assert!(matches!(err, StorageError::Sqlx(_)));
    }
}
