use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqlSafeStr, SqlitePool};

use super::rows::read_schema_meta;
use super::{IdempotencyCodec, StorageError, Store};
use crate::model::Clock;

pub const APPLICATION_ID: i64 = 1_397_248_068;
pub const SCHEMA_VERSION: i64 = 1;
pub const EXPORT_VERSION: &str = "2.0.0";

pub(crate) const BASELINE_SQL: &str = include_str!("../../migrations/20260914000001_baseline.sql");
const BASELINE_VERSION: i64 = 20_260_914_000_001;
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
// SQLite file format: 100-byte header, application_id big-endian at offset 68.
const HEADER_LEN: usize = 100;
const APPLICATION_ID_OFFSET: usize = 68;

pub struct StoreOptions {
    pub db_path: PathBuf,
    /// None resolves to the archived MVP default `~/.shepherd/shepherd.db`.
    pub mvp_db_path: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    pub codec: Arc<dyn IdempotencyCodec>,
}

pub async fn open(options: StoreOptions) -> Result<Store, StorageError> {
    let path = &options.db_path;
    if is_mvp_path(path, options.mvp_db_path.as_deref()) {
        return Err(StorageError::MvpDatabase { path: path.clone() });
    }
    let foreign = || StorageError::ForeignDatabase { path: path.clone() };

    // Foreign files are rejected from raw bytes: any SQLite open can create -shm/-wal siblings.
    let fresh = match probe_header(path)? {
        HeaderProbe::Missing | HeaderProbe::Empty => true,
        HeaderProbe::NotSqlite => return Err(foreign()),
        HeaderProbe::Sqlite { application_id } if application_id == APPLICATION_ID => false,
        HeaderProbe::Sqlite { .. } => return Err(foreign()),
    };

    if !fresh {
        // Reject before any write: never modify a database we will not open.
        match probe_schema_meta(path).await {
            SchemaProbe::V1 => {}
            SchemaProbe::Mismatch {
                version,
                export_version,
            } => {
                return Err(StorageError::SchemaMismatch {
                    version,
                    export_version,
                });
            }
            SchemaProbe::MissingMeta | SchemaProbe::Malformed => return Err(foreign()),
            SchemaProbe::Unreadable => {}
        }
    }

    let pool = build_pool(path).await?;
    let created = if fresh { file_identity(path) } else { None };
    if let Err(err) = initialize(&pool, path, fresh).await {
        pool.close().await;
        // Clean up our failed init; a ForeignDatabase is a foreign file we must not delete.
        if fresh && !matches!(err, StorageError::ForeignDatabase { .. }) {
            remove_unless_landed(path, created);
        }
        return Err(err);
    }
    Ok(Store::new(pool, options.clock, options.codec))
}

// Release targets are Unix only (plan/13), so dev+ino is a stable file identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

fn file_identity(path: &Path) -> Option<FileIdentity> {
    std::fs::metadata(path).ok().map(|meta| FileIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

// Keep the db if the close-time checkpoint landed; else remove it and its WAL sidecars.
// Only files this open created are removed: a replacement swapped into the path survives.
fn remove_unless_landed(path: &Path, created: Option<FileIdentity>) {
    let landed = matches!(
        probe_header(path),
        Ok(HeaderProbe::Sqlite { application_id }) if application_id == APPLICATION_ID
    );
    if landed {
        return;
    }
    if file_identity(path) != created {
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(Path::new(&sidecar));
    }
}

async fn initialize(pool: &SqlitePool, path: &Path, fresh: bool) -> Result<(), StorageError> {
    if fresh {
        ensure_empty_schema(pool, path).await?;
    }
    migrator().run(pool).await?;
    if fresh {
        checkpoint_identity(pool, path).await?;
    }
    verify_identity(pool).await
}

// A file appearing between the header probe and this write must not receive the baseline.
async fn ensure_empty_schema(pool: &SqlitePool, path: &Path) -> Result<(), StorageError> {
    let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master")
        .fetch_one(pool)
        .await?;
    if objects != 0 {
        return Err(StorageError::ForeignDatabase {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

// wal_checkpoint(TRUNCATE) reports contention via its result row, not an error.
async fn checkpoint_identity(pool: &SqlitePool, path: &Path) -> Result<(), StorageError> {
    let (busy, _, _) = sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(pool)
        .await?;
    let landed = busy == 0
        && matches!(
            probe_header(path)?,
            HeaderProbe::Sqlite { application_id } if application_id == APPLICATION_ID
        );
    if !landed {
        return Err(StorageError::Checkpoint {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn is_mvp_path(db_path: &Path, configured: Option<&Path>) -> bool {
    let mvp = match configured {
        Some(path) => path.to_path_buf(),
        None => match std::env::home_dir() {
            Some(home) => home.join(".shepherd").join("shepherd.db"),
            None => return false,
        },
    };
    if db_path == mvp {
        return true;
    }
    // Catch `..` segments and symlinks; both sides must exist to canonicalize.
    match (std::fs::canonicalize(db_path), std::fs::canonicalize(&mvp)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

enum HeaderProbe {
    Missing,
    Empty,
    NotSqlite,
    Sqlite { application_id: i64 },
}

fn probe_header(path: &Path) -> Result<HeaderProbe, StorageError> {
    let mut file = match std::fs::File::open(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HeaderProbe::Missing);
        }
        Err(err) => return Err(err.into()),
        Ok(file) => file,
    };
    let mut header = [0u8; HEADER_LEN];
    let mut filled = 0;
    while filled < HEADER_LEN {
        let read = file.read(&mut header[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    if filled == 0 {
        return Ok(HeaderProbe::Empty);
    }
    if filled < HEADER_LEN || !header.starts_with(SQLITE_MAGIC) {
        return Ok(HeaderProbe::NotSqlite);
    }
    let offset = APPLICATION_ID_OFFSET;
    let application_id = i32::from_be_bytes([
        header[offset],
        header[offset + 1],
        header[offset + 2],
        header[offset + 3],
    ]);
    Ok(HeaderProbe::Sqlite {
        application_id: i64::from(application_id),
    })
}

enum SchemaProbe {
    V1,
    Mismatch {
        version: i64,
        export_version: String,
    },
    MissingMeta,
    // Read-only open failed: our own db mid-WAL-recovery; the RW open recovers it.
    Unreadable,
    // Connected, but the metadata query or decode failed.
    Malformed,
}

async fn probe_schema_meta(path: &Path) -> SchemaProbe {
    let Ok(mut conn) = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
    else {
        return SchemaProbe::Unreadable;
    };
    let has_table = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
    )
    .fetch_one(&mut conn)
    .await;
    let probe = match has_table {
        Ok(0) => SchemaProbe::MissingMeta,
        Err(_) => SchemaProbe::Malformed,
        Ok(_) => {
            match sqlx::query_as::<_, (i64, String)>(
                "SELECT version, export_version FROM schema_meta",
            )
            .fetch_optional(&mut conn)
            .await
            {
                Ok(Some((version, export_version)))
                    if version == SCHEMA_VERSION && export_version == EXPORT_VERSION =>
                {
                    SchemaProbe::V1
                }
                Ok(Some((version, export_version))) => SchemaProbe::Mismatch {
                    version,
                    export_version,
                },
                Ok(None) => SchemaProbe::MissingMeta,
                Err(_) => SchemaProbe::Malformed,
            }
        }
    };
    conn.close().await.ok();
    probe
}

async fn build_pool(path: &Path) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5000));
    Ok(SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?)
}

fn migrator() -> Migrator {
    Migrator::with_migrations(vec![Migration::new(
        BASELINE_VERSION,
        "baseline".into(),
        MigrationType::Simple,
        BASELINE_SQL.into_sql_str(),
        false,
    )])
}

async fn verify_identity(pool: &SqlitePool) -> Result<(), StorageError> {
    let id = sqlx::query_scalar::<_, i64>("PRAGMA application_id")
        .fetch_one(pool)
        .await?;
    let meta = read_schema_meta(pool).await?;
    if id != APPLICATION_ID
        || meta.version != SCHEMA_VERSION
        || meta.export_version != EXPORT_VERSION
    {
        return Err(StorageError::SchemaMismatch {
            version: meta.version,
            export_version: meta.export_version,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::testing::{store_options, test_clock};

    async fn pragma_i64(pool: &SqlitePool, pragma: &'static str) -> i64 {
        sqlx::query_scalar(pragma).fetch_one(pool).await.unwrap()
    }

    fn dir_snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut entries: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        entries.sort();
        entries
    }

    async fn create_foreign_db(path: &Path, wal: bool) {
        let mut options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        if wal {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        let mut conn = options.connect().await.unwrap();
        sqlx::raw_sql("CREATE TABLE legacy(x TEXT); INSERT INTO legacy VALUES('data');")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
    }

    #[tokio::test]
    async fn fresh_database_initializes_identity_and_sqlite_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let pool = store.pool();

        assert_eq!(
            pragma_i64(pool, "PRAGMA application_id").await,
            APPLICATION_ID
        );
        assert_eq!(pragma_i64(pool, "PRAGMA foreign_keys").await, 1);
        assert_eq!(pragma_i64(pool, "PRAGMA synchronous").await, 2);
        assert_eq!(pragma_i64(pool, "PRAGMA busy_timeout").await, 5000);
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(journal, "wal");

        let meta = read_schema_meta(pool).await.unwrap();
        assert_eq!(meta.version, SCHEMA_VERSION);
        assert_eq!(meta.export_version, EXPORT_VERSION);
        let lock: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(lock, 0);
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name != '_sqlx_migrations' \
             ORDER BY name",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        let expected = [
            "actors",
            "browser_sessions",
            "claims",
            "command_lock",
            "credentials",
            "document_revisions",
            "documents",
            "epic_dependencies",
            "epics",
            "events",
            "goals",
            "idempotency",
            "projects",
            "reviews",
            "schema_meta",
            "security_audit",
            "sessions",
            "submissions",
            "task_dependencies",
            "task_types",
            "tasks",
        ];
        assert_eq!(tables, expected);
        let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(applied, 1);
    }

    #[tokio::test]
    async fn fresh_database_header_carries_identity_before_close() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let probe = probe_header(&dir.path().join("shepherd.db")).unwrap();
        assert!(matches!(
            probe,
            HeaderProbe::Sqlite { application_id } if application_id == APPLICATION_ID
        ));
        store.pool().close().await;
    }

    #[tokio::test]
    async fn contended_checkpoint_fails_loudly_and_lands_when_readers_release() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shepherd.db");
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let mut reader = SqliteConnectOptions::new()
            .filename(&db_path)
            .read_only(true)
            .connect()
            .await
            .unwrap();
        sqlx::query("BEGIN").execute(&mut reader).await.unwrap();
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sqlite_master")
            .fetch_one(&mut reader)
            .await
            .unwrap();
        // value=value would be optimized to a no-op with no WAL frame.
        sqlx::query("UPDATE command_lock SET value=value+1 WHERE id=1")
            .execute(store.pool())
            .await
            .unwrap();

        // busy_timeout waits the full 5s before contention is reported.
        let err = checkpoint_identity(store.pool(), &db_path)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Checkpoint { .. }));

        sqlx::query("COMMIT").execute(&mut reader).await.unwrap();
        reader.close().await.unwrap();
        checkpoint_identity(store.pool(), &db_path).await.unwrap();
        store.pool().close().await;
    }

    #[tokio::test]
    async fn fresh_open_rejects_a_file_that_appeared_mid_open() {
        // The race cannot be won deterministically; the guard is proven directly.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("late.db");
        create_foreign_db(&path, false).await;
        let pool = build_pool(&path).await.unwrap();
        let Err(err) = ensure_empty_schema(&pool, &path).await else {
            panic!("expected mid-open file rejection")
        };
        assert!(matches!(err, StorageError::ForeignDatabase { path: p } if p == path));
        pool.close().await;
    }

    #[tokio::test]
    async fn empty_file_initializes_fresh_database() {
        let dir = tempfile::tempdir().unwrap();
        let options = store_options(dir.path(), "shepherd.db", test_clock());
        std::fs::write(&options.db_path, b"").unwrap();
        let store = open(options).await.unwrap();
        assert_eq!(
            pragma_i64(store.pool(), "PRAGMA application_id").await,
            APPLICATION_ID
        );
    }

    #[tokio::test]
    async fn reopening_existing_database_preserves_state() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            sqlx::query("UPDATE command_lock SET value=42 WHERE id=1")
                .execute(store.pool())
                .await
                .unwrap();
            store.pool().close().await;
        }
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let lock: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(lock, 42);
    }

    #[tokio::test]
    async fn crashed_v1_database_recovers_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        sqlx::query("UPDATE command_lock SET value=7 WHERE id=1")
            .execute(store.pool())
            .await
            .unwrap();

        // Crash simulation: db + hot WAL survive without the -shm index.
        let crash_dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            dir.path().join("shepherd.db"),
            crash_dir.path().join("shepherd.db"),
        )
        .unwrap();
        std::fs::copy(
            dir.path().join("shepherd.db-wal"),
            crash_dir.path().join("shepherd.db-wal"),
        )
        .unwrap();
        store.pool().close().await;

        let recovered = open(store_options(crash_dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let lock: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
            .fetch_one(recovered.pool())
            .await
            .unwrap();
        assert_eq!(lock, 7);
    }

    #[tokio::test]
    async fn mvp_database_path_is_rejected_without_io() {
        let dir = tempfile::tempdir().unwrap();
        let mvp = dir.path().join("mvp").join("shepherd.db");
        let mut options = store_options(dir.path(), "ignored.db", test_clock());
        options.db_path = mvp.clone();
        options.mvp_db_path = Some(mvp.clone());
        let Err(err) = open(options).await else {
            panic!("expected MVP path rejection")
        };
        assert!(matches!(err, StorageError::MvpDatabase { path } if path == mvp));
        assert!(!mvp.exists());
    }

    #[tokio::test]
    async fn mvp_database_indirect_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mvp_dir = dir.path().join("mvp");
        std::fs::create_dir_all(&mvp_dir).unwrap();
        let mvp = mvp_dir.join("shepherd.db");
        std::fs::write(&mvp, b"legacy bytes").unwrap();
        let indirect = dir
            .path()
            .join("mvp")
            .join("..")
            .join("mvp")
            .join("shepherd.db");
        let mut options = store_options(dir.path(), "ignored.db", test_clock());
        options.db_path = indirect;
        options.mvp_db_path = Some(mvp);
        let Err(err) = open(options).await else {
            panic!("expected MVP path rejection")
        };
        assert!(matches!(err, StorageError::MvpDatabase { .. }));
        assert_eq!(
            std::fs::read(dir.path().join("mvp").join("shepherd.db")).unwrap(),
            b"legacy bytes"
        );
    }

    async fn assert_rejected_without_byte_changes(dir: &Path, db_name: &str) {
        let options = store_options(dir, db_name, test_clock());
        let path = options.db_path.clone();
        let before = dir_snapshot(dir);
        let Err(err) = open(options).await else {
            panic!("expected foreign database rejection")
        };
        assert!(matches!(err, StorageError::ForeignDatabase { path: p } if p == path));
        assert_eq!(dir_snapshot(dir), before);
    }

    #[tokio::test]
    async fn foreign_sqlite_database_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        create_foreign_db(&dir.path().join("legacy.db"), false).await;
        assert_rejected_without_byte_changes(dir.path(), "legacy.db").await;
    }

    #[tokio::test]
    async fn foreign_wal_database_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        create_foreign_db(&dir.path().join("legacy.db"), true).await;
        assert_rejected_without_byte_changes(dir.path(), "legacy.db").await;
    }

    #[tokio::test]
    async fn foreign_database_with_stray_wal_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        create_foreign_db(&dir.path().join("legacy.db"), false).await;
        std::fs::write(dir.path().join("legacy.db-wal"), b"stray wal bytes").unwrap();
        assert_rejected_without_byte_changes(dir.path(), "legacy.db").await;
    }

    #[tokio::test]
    async fn non_sqlite_file_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.db"), b"not a database at all").unwrap();
        assert_rejected_without_byte_changes(dir.path(), "notes.db").await;
    }

    #[tokio::test]
    async fn truncated_sqlite_header_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stub.db"), SQLITE_MAGIC).unwrap();
        assert_rejected_without_byte_changes(dir.path(), "stub.db").await;
    }

    #[tokio::test]
    async fn matching_id_without_schema_meta_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claimed.db");
        create_foreign_db(&path, false).await;
        let mut conn = SqliteConnectOptions::new()
            .filename(&path)
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql("PRAGMA application_id=1397248068;")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        assert_rejected_without_byte_changes(dir.path(), "claimed.db").await;
    }

    #[tokio::test]
    async fn matching_id_with_malformed_schema_meta_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claimed.db");
        create_foreign_db(&path, false).await;
        let mut conn = SqliteConnectOptions::new()
            .filename(&path)
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql("PRAGMA application_id=1397248068; CREATE TABLE schema_meta(junk TEXT);")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        assert_rejected_without_byte_changes(dir.path(), "claimed.db").await;
    }

    #[tokio::test]
    async fn matching_id_with_corrupt_schema_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            store.pool().close().await;
            std::fs::copy(dir.path().join("shepherd.db"), &path).unwrap();
        }
        // Break sqlite_master's root page (offset 100), keeping the file header intact.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[100] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();

        // Only the data file is byte-compared: the read-only probe may leave -shm/-wal siblings.
        let before = std::fs::read(&path).unwrap();
        let Err(err) = open(store_options(dir.path(), "corrupt.db", test_clock())).await else {
            panic!("expected corrupt schema rejection")
        };
        assert!(matches!(err, StorageError::ForeignDatabase { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[tokio::test]
    async fn remove_unless_landed_keeps_a_landed_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shepherd.db");
        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        store.pool().close().await;
        remove_unless_landed(&db_path, file_identity(&db_path));
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn remove_unless_landed_clears_an_unlanded_init_for_a_clean_retry() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shepherd.db");
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            store.pool().close().await;
        }
        // Simulate a failed init: baseline applied, raw header still without the id.
        let mut bytes = std::fs::read(&db_path).unwrap();
        bytes[APPLICATION_ID_OFFSET..APPLICATION_ID_OFFSET + 4].fill(0);
        std::fs::write(&db_path, bytes).unwrap();
        std::fs::write(dir.path().join("shepherd.db-wal"), b"stray wal").unwrap();
        std::fs::write(dir.path().join("shepherd.db-shm"), b"stray shm").unwrap();

        remove_unless_landed(&db_path, file_identity(&db_path));
        assert!(!db_path.exists());
        assert!(!dir.path().join("shepherd.db-wal").exists());
        assert!(!dir.path().join("shepherd.db-shm").exists());

        let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap();
        let probe = probe_header(&db_path).unwrap();
        assert!(matches!(
            probe,
            HeaderProbe::Sqlite { application_id } if application_id == APPLICATION_ID
        ));
        store.pool().close().await;
    }

    #[tokio::test]
    async fn remove_unless_landed_spares_a_replacement_file() {
        // The race cannot be won deterministically; the guard is proven directly.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shepherd.db");
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            store.pool().close().await;
        }
        let created = file_identity(&db_path);
        assert!(created.is_some());

        // A foreign file swapped into the path after our init must survive cleanup.
        std::fs::write(dir.path().join("foreign.db"), b"foreign bytes").unwrap();
        let foreign_identity = file_identity(&dir.path().join("foreign.db")).unwrap();
        std::fs::rename(dir.path().join("foreign.db"), &db_path).unwrap();

        remove_unless_landed(&db_path, created);
        assert_eq!(file_identity(&db_path), Some(foreign_identity));
        assert_eq!(std::fs::read(&db_path).unwrap(), b"foreign bytes".to_vec());
    }

    #[tokio::test]
    async fn remove_unless_landed_spares_files_it_never_proved_owning() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shepherd.db");
        std::fs::write(dir.path().join("shepherd.db-wal"), b"stray wal").unwrap();

        // An unlanded db without a captured identity is not provably ours.
        std::fs::write(&db_path, SQLITE_MAGIC).unwrap();
        remove_unless_landed(&db_path, None);
        assert!(db_path.exists());
        assert!(dir.path().join("shepherd.db-wal").exists());

        // The db missing from the path: surviving sidecars are not provably ours either.
        let identity = file_identity(&db_path).unwrap();
        std::fs::remove_file(&db_path).unwrap();
        remove_unless_landed(&db_path, Some(identity));
        assert!(dir.path().join("shepherd.db-wal").exists());
    }

    #[tokio::test]
    async fn contended_fresh_init_is_cleaned_up_and_retryable() {
        // The reader must snapshot after the WAL conversion (earlier would block
        // build_pool) but before the baseline commit, so it cannot be scheduled
        // deterministically; retry the whole first-open until it truly contends.
        for _ in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("shepherd.db");
            let wal_path = dir.path().join("shepherd.db-wal");
            let reader = tokio::spawn({
                let (db_path, wal_path) = (db_path.clone(), wal_path.clone());
                async move {
                    while !wal_path.exists() {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    let mut conn = SqliteConnectOptions::new()
                        .filename(&db_path)
                        .read_only(true)
                        .connect()
                        .await
                        .unwrap();
                    sqlx::query("BEGIN").execute(&mut conn).await.unwrap();
                    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sqlite_master")
                        .fetch_one(&mut conn)
                        .await
                        .unwrap();
                    // Hold the snapshot far beyond the checkpoint's busy_timeout.
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            });

            match open(store_options(dir.path(), "shepherd.db", test_clock())).await {
                Err(StorageError::Checkpoint { .. }) => {
                    reader.abort();
                    assert!(!db_path.exists());
                    assert!(!wal_path.exists());
                    assert!(!dir.path().join("shepherd.db-shm").exists());

                    let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                        .await
                        .unwrap();
                    let probe = probe_header(&db_path).unwrap();
                    assert!(matches!(
                        probe,
                        HeaderProbe::Sqlite { application_id } if application_id == APPLICATION_ID
                    ));
                    store.pool().close().await;
                    return;
                }
                Ok(store) => {
                    store.pool().close().await;
                    reader.abort();
                }
                Err(other) => panic!("unexpected open error: {other:?}"),
            }
        }
        panic!("could not trigger checkpoint contention in 5 attempts");
    }

    #[tokio::test]
    async fn newer_schema_meta_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            store.pool().close().await;
        }
        let mut conn = SqliteConnectOptions::new()
            .filename(dir.path().join("shepherd.db"))
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql("UPDATE schema_meta SET version=2, export_version='3.0.0';")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();

        let db_path = dir.path().join("shepherd.db");
        let before = std::fs::read(&db_path).unwrap();
        let Err(err) = open(store_options(dir.path(), "shepherd.db", test_clock())).await else {
            panic!("expected schema mismatch rejection")
        };
        assert!(matches!(
            err,
            StorageError::SchemaMismatch { version: 2, export_version } if export_version == "3.0.0"
        ));
        // Only the data file is byte-compared: the read-only probe may leave -shm/-wal siblings.
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
    }

    #[tokio::test]
    async fn unknown_future_migration_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(store_options(dir.path(), "shepherd.db", test_clock()))
                .await
                .unwrap();
            store.pool().close().await;
        }
        let mut conn = SqliteConnectOptions::new()
            .filename(dir.path().join("shepherd.db"))
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (99990101000001, 'future', CURRENT_TIMESTAMP, TRUE, x'00', 0);",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        conn.close().await.unwrap();

        let Err(err) = open(store_options(dir.path(), "shepherd.db", test_clock())).await else {
            panic!("expected future migration rejection")
        };
        assert!(matches!(err, StorageError::Migrate(_)));
    }
}
