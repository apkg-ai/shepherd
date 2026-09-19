use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use shepherd_core::model::{Actor, ActorId, ActorKind, Clock, CommandId, TestClock};
use shepherd_core::storage::rows::{
    IdempotencyRecord, get_actor, get_idempotency, insert_actor, put_idempotency,
};
use shepherd_core::storage::{
    APPLICATION_ID, EXPORT_VERSION, IdempotencyAad, SCHEMA_VERSION, SealedResponse, StorageError,
    Store, StoreOptions, TestCodec, TestKeyProvider, open,
};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use uuid::Uuid;

fn start_time() -> DateTime<Utc> {
    "2026-09-14T00:00:00Z".parse().unwrap()
}

fn options(dir: &tempfile::TempDir, db_name: &str, clock: Arc<TestClock>) -> StoreOptions {
    StoreOptions {
        db_path: dir.path().join(db_name),
        mvp_db_path: Some(dir.path().join("mvp").join("shepherd.db")),
        clock,
        codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([3; 32])))),
    }
}

async fn open_store(dir: &tempfile::TempDir, clock: Arc<TestClock>) -> Store {
    open(options(dir, "shepherd.db", clock)).await.unwrap()
}

// Persisted-state assertions go through an independent read-only connection,
// not the Store under test.
async fn independent_connection(path: &Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .unwrap()
}

fn actor(now: DateTime<Utc>, label: &str) -> Actor {
    Actor {
        id: ActorId::generate(now),
        kind: ActorKind::Agent,
        label: label.to_string(),
        revoked: false,
        created_at: now,
    }
}

#[tokio::test]
async fn fresh_database_opens_with_v1_identity() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    let store = open_store(&dir, clock).await;
    let db_path = dir.path().join("shepherd.db");

    let mut conn = independent_connection(&db_path).await;
    let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(application_id, APPLICATION_ID);
    let (version, export_version): (i64, String) =
        sqlx::query_as("SELECT version, export_version FROM schema_meta")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(export_version, EXPORT_VERSION);
    let lock_value: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(lock_value, 0);
    let baseline_applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version=20260914000001")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(baseline_applied, 1);
    conn.close().await.unwrap();
    drop(store);
}

#[tokio::test]
async fn reopening_v1_database_preserves_data_without_new_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    let registered = actor(clock.now(), "kept-agent");
    {
        let store = open_store(&dir, clock.clone()).await;
        let inserted = registered.clone();
        store
            .command_transaction(|tx| {
                Box::pin(async move { insert_actor(&mut **tx, &inserted).await })
            })
            .await
            .unwrap();
        store.pool().close().await;
    }
    let store = open_store(&dir, clock).await;
    assert_eq!(
        get_actor(store.pool(), &registered.id).await.unwrap(),
        Some(registered)
    );
    let migrations: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(migrations, 1);
}

async fn assert_rejection_leaves_bytes_untouched(dir: &tempfile::TempDir, db_name: &str) {
    let clock = Arc::new(TestClock::new(start_time()));
    let opts = options(dir, db_name, clock);
    let path = opts.db_path.clone();
    let before = std::fs::read(&path).unwrap();

    let Err(err) = open(opts).await else {
        panic!("expected rejection of {db_name}");
    };
    assert!(matches!(err, StorageError::ForeignDatabase { path: p } if p == path));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec![db_name.to_string()]);
}

#[tokio::test]
async fn foreign_nonempty_database_is_rejected_without_modified_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let mut conn = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE mvp_tasks(id TEXT); INSERT INTO mvp_tasks VALUES('t-1');")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    assert_rejection_leaves_bytes_untouched(&dir, "legacy.db").await;
}

#[tokio::test]
async fn non_sqlite_file_is_rejected_without_modified_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.db"), b"plain text, not a database").unwrap();
    assert_rejection_leaves_bytes_untouched(&dir, "notes.db").await;
}

#[tokio::test]
async fn mvp_database_path_is_rejected_without_open() {
    let dir = tempfile::tempdir().unwrap();
    let mvp: PathBuf = dir
        .path()
        .join("mvp-home")
        .join(".shepherd")
        .join("shepherd.db");
    let clock = Arc::new(TestClock::new(start_time()));
    let opts = StoreOptions {
        db_path: mvp.clone(),
        mvp_db_path: Some(mvp.clone()),
        clock,
        codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([3; 32])))),
    };
    let Err(err) = open(opts).await else {
        panic!("expected MVP rejection");
    };
    assert!(matches!(err, StorageError::MvpDatabase { path } if path == mvp));
    assert!(!mvp.exists());
}

#[tokio::test]
async fn failure_injected_after_insert_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    let store = open_store(&dir, clock.clone()).await;
    let db_path = dir.path().join("shepherd.db");

    // Independently committed prior command must survive the later rollback.
    let prior = actor(clock.now(), "prior");
    let prior_insert = prior.clone();
    store
        .command_transaction(|tx| {
            Box::pin(async move { insert_actor(&mut **tx, &prior_insert).await })
        })
        .await
        .unwrap();

    let failing = actor(clock.now(), "failing");
    let failing_insert = failing.clone();
    let now = clock.now();
    let Err(err) = store
        .command_transaction(|tx| {
            Box::pin(async move {
                insert_actor(&mut **tx, &failing_insert).await?;
                put_idempotency(
                    &mut **tx,
                    &IdempotencyRecord {
                        actor_id: failing_insert.id,
                        key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
                        command_id: CommandId::generate(now),
                        request_hash: "hash-rollback".to_string(),
                        response_status: 201,
                        sealed: SealedResponse {
                            ciphertext: vec![1],
                            nonce: vec![0; 12],
                        },
                        created_at: now,
                        expires_at: now + TimeDelta::days(7),
                    },
                )
                .await?;
                Err::<(), _>(StorageError::Corrupt("injected failure".into()))
            })
        })
        .await
    else {
        panic!("expected injected failure");
    };
    assert!(matches!(err, StorageError::Corrupt(_)));

    let mut conn = independent_connection(&db_path).await;
    let actors: Vec<String> = sqlx::query_scalar("SELECT label FROM actors ORDER BY label")
        .fetch_all(&mut conn)
        .await
        .unwrap();
    assert_eq!(actors, vec!["prior".to_string()]);
    let idempotency_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(idempotency_rows, 0);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn two_file_backed_pools_serialize_commands() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    // The second open also exercises the identity probe against a live WAL database.
    let store_a = Arc::new(open_store(&dir, clock.clone()).await);
    let store_b = Arc::new(open_store(&dir, clock).await);

    let in_flight = Arc::new(AtomicI64::new(0));
    let max_in_flight = Arc::new(AtomicI64::new(0));
    let mut handles = Vec::new();
    for store in [store_a.clone(), store_b.clone()] {
        for _ in 0..12 {
            let store = store.clone();
            let in_flight = in_flight.clone();
            let max_in_flight = max_in_flight.clone();
            handles.push(tokio::spawn(async move {
                store
                    .command_transaction(|tx| {
                        Box::pin(async move {
                            let concurrent = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                            max_in_flight.fetch_max(concurrent, Ordering::SeqCst);
                            let value: i64 =
                                sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
                                    .fetch_one(&mut **tx)
                                    .await?;
                            // Widen the read-modify-write window so lost updates would surface.
                            tokio::time::sleep(Duration::from_millis(2)).await;
                            sqlx::query("UPDATE command_lock SET value=?1 WHERE id=1")
                                .bind(value + 1)
                                .execute(&mut **tx)
                                .await?;
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        })
                    })
                    .await
            }));
        }
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);
    let final_value: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
        .fetch_one(store_a.pool())
        .await
        .unwrap();
    assert_eq!(final_value, 24);
}

#[tokio::test]
async fn idempotency_codec_round_trips_and_rejects_tampered_aad() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    let store = open_store(&dir, clock.clone()).await;

    let agent = actor(clock.now(), "sealer");
    insert_actor(store.pool(), &agent).await.unwrap();
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let aad = IdempotencyAad {
        actor_id: &agent.id,
        key: &key,
        request_hash: "hash-seal",
    };
    let sealed = store.codec().seal(&aad, b"claim grant body").unwrap();
    assert_ne!(sealed.ciphertext, b"claim grant body");

    let now = clock.now();
    put_idempotency(
        store.pool(),
        &IdempotencyRecord {
            actor_id: agent.id,
            key,
            command_id: CommandId::generate(now),
            request_hash: "hash-seal".to_string(),
            response_status: 201,
            sealed,
            created_at: now,
            expires_at: now + TimeDelta::days(7),
        },
    )
    .await
    .unwrap();

    let stored = get_idempotency(store.pool(), &agent.id, &key, clock.now())
        .await
        .unwrap()
        .expect("stored replay row");
    assert_eq!(
        store.codec().open(&aad, &stored.sealed).unwrap(),
        b"claim grant body"
    );

    let wrong_aad = IdempotencyAad {
        actor_id: &agent.id,
        key: &key,
        request_hash: "hash-other",
    };
    assert!(matches!(
        store.codec().open(&wrong_aad, &stored.sealed),
        Err(StorageError::Codec(_))
    ));
}

#[tokio::test]
async fn idempotency_lookup_honors_ttl_with_test_clock() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(start_time()));
    let store = open_store(&dir, clock.clone()).await;

    let agent = actor(clock.now(), "replayer");
    insert_actor(store.pool(), &agent).await.unwrap();
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let now = clock.now();
    put_idempotency(
        store.pool(),
        &IdempotencyRecord {
            actor_id: agent.id,
            key,
            command_id: CommandId::generate(now),
            request_hash: "hash-ttl".to_string(),
            response_status: 200,
            sealed: SealedResponse {
                ciphertext: vec![5],
                nonce: vec![0; 12],
            },
            created_at: now,
            expires_at: now + TimeDelta::days(7),
        },
    )
    .await
    .unwrap();

    clock.advance(TimeDelta::days(7) - TimeDelta::milliseconds(1));
    assert!(
        get_idempotency(store.pool(), &agent.id, &key, clock.now())
            .await
            .unwrap()
            .is_some()
    );
    clock.advance(TimeDelta::milliseconds(1));
    assert!(
        get_idempotency(store.pool(), &agent.id, &key, clock.now())
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn baseline_migration_matches_contract_schema() {
    fn normalized(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
            .map(str::to_string)
            .collect()
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let migration = normalized(&manifest.join("migrations/20260914000001_baseline.sql"));
    let contract = normalized(&manifest.join("../../plan/contracts/schema.sql"));
    assert!(!migration.is_empty());
    assert_eq!(migration, contract);
}
