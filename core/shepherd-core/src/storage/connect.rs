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
const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";

pub struct StoreOptions {
    pub db_path: PathBuf,
    /// None resolves to the archived MVP default `~/.shepherd/shepherd.db`.
    pub mvp_db_path: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    pub codec: Arc<dyn IdempotencyCodec>,
}

pub async fn open(options: StoreOptions) -> Result<Store, StorageError> {
    let path = &options.db_path;
    if Some(path.as_path()) == mvp_db_path(options.mvp_db_path.as_deref()).as_deref() {
        return Err(StorageError::MvpDatabase { path: path.clone() });
    }
    match probe_header(path)? {
        HeaderProbe::Missing | HeaderProbe::Empty => {}
        HeaderProbe::NotSqlite => {
            return Err(StorageError::ForeignDatabase { path: path.clone() });
        }
        HeaderProbe::Sqlite => {
            if probe_application_id(path).await? != APPLICATION_ID {
                return Err(StorageError::ForeignDatabase { path: path.clone() });
            }
        }
    }
    let pool = build_pool(path).await?;
    migrator().run(&pool).await?;
    verify_identity(&pool).await?;
    Ok(Store::new(pool, options.clock, options.codec))
}

fn mvp_db_path(configured: Option<&Path>) -> Option<PathBuf> {
    match configured {
        Some(path) => Some(path.to_path_buf()),
        None => std::env::home_dir().map(|home| home.join(".shepherd").join("shepherd.db")),
    }
}

enum HeaderProbe {
    Missing,
    Empty,
    NotSqlite,
    Sqlite,
}

fn probe_header(path: &Path) -> Result<HeaderProbe, StorageError> {
    match std::fs::read(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HeaderProbe::Missing),
        Err(err) => Err(err.into()),
        Ok(bytes) if bytes.is_empty() => Ok(HeaderProbe::Empty),
        Ok(bytes) if bytes.starts_with(SQLITE_HEADER) => Ok(HeaderProbe::Sqlite),
        Ok(_) => Ok(HeaderProbe::NotSqlite),
    }
}

// Read-only probe: reads through a live WAL and never writes; any failure means
// the file is not a healthy v1 database, so callers reject without touching it.
async fn probe_application_id(path: &Path) -> Result<i64, StorageError> {
    let foreign = || StorageError::ForeignDatabase {
        path: path.to_path_buf(),
    };
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .map_err(|_| foreign())?;
    let id = sqlx::query_scalar::<_, i64>("PRAGMA application_id")
        .fetch_one(&mut conn)
        .await
        .map_err(|_| foreign());
    conn.close().await.ok();
    id
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
    use crate::model::TestClock;
    use crate::storage::{TestCodec, TestKeyProvider};

    fn test_options(dir: &tempfile::TempDir, db_name: &str) -> StoreOptions {
        StoreOptions {
            db_path: dir.path().join(db_name),
            mvp_db_path: Some(dir.path().join("mvp").join("shepherd.db")),
            clock: Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap())),
            codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([0; 32])))),
        }
    }

    async fn pragma_i64(pool: &SqlitePool, pragma: &'static str) -> i64 {
        sqlx::query_scalar(pragma).fetch_one(pool).await.unwrap()
    }

    #[tokio::test]
    async fn fresh_database_initializes_identity_and_sqlite_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(test_options(&dir, "shepherd.db")).await.unwrap();
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
    async fn empty_file_initializes_fresh_database() {
        let dir = tempfile::tempdir().unwrap();
        let options = test_options(&dir, "shepherd.db");
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
            let store = open(test_options(&dir, "shepherd.db")).await.unwrap();
            sqlx::query("UPDATE command_lock SET value=42 WHERE id=1")
                .execute(store.pool())
                .await
                .unwrap();
            store.pool().close().await;
        }
        let store = open(test_options(&dir, "shepherd.db")).await.unwrap();
        let lock: i64 = sqlx::query_scalar("SELECT value FROM command_lock WHERE id=1")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(lock, 42);
    }

    #[tokio::test]
    async fn mvp_database_path_is_rejected_without_io() {
        let dir = tempfile::tempdir().unwrap();
        let mvp = dir.path().join("mvp").join("shepherd.db");
        let options = StoreOptions {
            db_path: mvp.clone(),
            mvp_db_path: Some(mvp.clone()),
            clock: Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap())),
            codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([0; 32])))),
        };
        let Err(err) = open(options).await else {
            panic!("expected MVP path rejection")
        };
        assert!(matches!(err, StorageError::MvpDatabase { path } if path == mvp));
        assert!(!mvp.exists());
    }

    async fn assert_rejected_without_byte_changes(dir: &tempfile::TempDir, db_name: &str) {
        let options = test_options(dir, db_name);
        let path = options.db_path.clone();
        let before = std::fs::read(&path).unwrap();
        let Err(err) = open(options).await else {
            panic!("expected foreign database rejection")
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
    async fn foreign_sqlite_database_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let mut conn = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE legacy(x TEXT); INSERT INTO legacy VALUES('data');")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        assert_rejected_without_byte_changes(&dir, "legacy.db").await;
    }

    #[tokio::test]
    async fn non_sqlite_file_is_rejected_without_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.db"), b"not a database at all").unwrap();
        assert_rejected_without_byte_changes(&dir, "notes.db").await;
    }
}
