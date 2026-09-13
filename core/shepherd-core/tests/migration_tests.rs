//! Migration tests: verify schema creation, FK constraints, and CHECK
//! constraints against a fresh SQLite database — plus the epic-type
//! upgrade path (the tasks-table rebuild runs outside sqlx's migrator,
//! so it needs a populated pre-epic database to be exercised at all).

use shepherd_core::Store;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

#[tokio::test]
async fn migrations_create_all_tables() {
    let store = Store::new_in_memory().await.unwrap();

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();

    assert!(
        tables.contains(&"projects".into()),
        "missing projects table"
    );
    assert!(tables.contains(&"tasks".into()), "missing tasks table");
    assert!(
        tables.contains(&"relations".into()),
        "missing relations table"
    );
    assert!(tables.contains(&"claims".into()), "missing claims table");
    assert!(
        tables.contains(&"sessions".into()),
        "missing sessions table"
    );
    assert!(
        tables.contains(&"knowledge_items".into()),
        "missing knowledge_items table"
    );
}

#[tokio::test]
async fn migrations_create_indexes() {
    let store = Store::new_in_memory().await.unwrap();

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%' ORDER BY name",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();

    assert!(
        indexes.contains(&"idx_projects_listing".into()),
        "missing projects listing index"
    );
    assert!(
        indexes.contains(&"idx_tasks_by_project".into()),
        "missing tasks by project index"
    );
    assert!(
        indexes.contains(&"idx_relations_source".into()),
        "missing relations source index"
    );
    assert!(
        indexes.contains(&"idx_claims_active".into()),
        "missing claims active index"
    );
    assert!(
        indexes.contains(&"idx_sessions_by_task".into()),
        "missing sessions by task index"
    );
    assert!(
        indexes.contains(&"idx_knowledge_by_project".into()),
        "missing knowledge by project index"
    );
    assert!(
        indexes.contains(&"idx_claims_one_active".into()),
        "missing one-active-claim unique index"
    );
}

#[tokio::test]
async fn second_active_claim_rejected() {
    let store = Store::new_in_memory().await.unwrap();
    let pool = store.pool();

    seed_project_and_task(pool, "p1", "t1").await;

    sqlx::query(
        "INSERT INTO claims (id, task_id, identity, ttl_seconds, lease_id, acquired_at, expires_at)
         VALUES ('c1', 't1', '{}', 300, 'l1', '2026-09-07T00:00:00Z', '2026-09-07T00:05:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();

    let result = sqlx::query(
        "INSERT INTO claims (id, task_id, identity, ttl_seconds, lease_id, acquired_at, expires_at)
         VALUES ('c2', 't1', '{}', 300, 'l2', '2026-09-07T00:01:00Z', '2026-09-07T00:06:00Z')",
    )
    .execute(pool)
    .await;

    assert!(
        result.is_err(),
        "should reject second active claim per task"
    );
}

#[tokio::test]
async fn released_claims_do_not_conflict() {
    let store = Store::new_in_memory().await.unwrap();
    let pool = store.pool();

    seed_project_and_task(pool, "p1", "t1").await;

    sqlx::query(
        "INSERT INTO claims (id, task_id, identity, ttl_seconds, lease_id, acquired_at, expires_at, released_at, release_reason)
         VALUES ('c1', 't1', '{}', 300, 'l1', '2026-09-07T00:00:00Z', '2026-09-07T00:05:00Z', '2026-09-07T00:02:00Z', 'voluntary')",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO claims (id, task_id, identity, ttl_seconds, lease_id, acquired_at, expires_at)
         VALUES ('c2', 't1', '{}', 300, 'l2', '2026-09-07T00:03:00Z', '2026-09-07T00:08:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn check_constraint_rejects_invalid_task_status() {
    let store = Store::new_in_memory().await.unwrap();

    seed_project(store.pool(), "p1").await;

    let result = sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES ('t1', 'p1', 'bad', 'code', 'INVALID', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .execute(store.pool())
    .await;

    assert!(result.is_err(), "should reject invalid task status");
}

#[tokio::test]
async fn check_constraint_rejects_invalid_task_type() {
    let store = Store::new_in_memory().await.unwrap();

    seed_project(store.pool(), "p1").await;

    let result = sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES ('t1', 'p1', 'bad', 'BADTYPE', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .execute(store.pool())
    .await;

    assert!(result.is_err(), "should reject invalid task type");
}

#[tokio::test]
async fn fk_constraint_rejects_orphan_task() {
    let store = Store::new_in_memory().await.unwrap();

    let result = sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES ('t1', 'nonexistent-project', 'orphan', 'code', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .execute(store.pool())
    .await;

    assert!(result.is_err(), "should reject FK to nonexistent project");
}

#[tokio::test]
async fn relation_self_loop_rejected() {
    let store = Store::new_in_memory().await.unwrap();
    let pool = store.pool();

    seed_project_and_task(pool, "p1", "t1").await;

    let result = sqlx::query(
        "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
         VALUES ('r1', 'depends_on', 't1', 't1', '2026-09-07T00:00:00Z')",
    )
    .execute(pool)
    .await;

    assert!(result.is_err(), "should reject self-loop relation");
}

#[tokio::test]
async fn relation_duplicate_rejected() {
    let store = Store::new_in_memory().await.unwrap();
    let pool = store.pool();

    seed_project_and_tasks(pool, "p1", &["t1", "t2"]).await;

    sqlx::query(
        "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
         VALUES ('r1', 'depends_on', 't1', 't2', '2026-09-07T00:00:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();

    let result = sqlx::query(
        "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
         VALUES ('r2', 'depends_on', 't1', 't2', '2026-09-07T00:00:00Z')",
    )
    .execute(pool)
    .await;

    assert!(result.is_err(), "should reject duplicate relation");
}

#[tokio::test]
async fn seeded_data_survives_remigration() {
    let store = Store::new_in_memory().await.unwrap();
    let pool = store.pool();

    seed_project_and_task(pool, "p1", "t1").await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // Run migrations again (idempotent).
    sqlx::migrate!("./migrations").run(pool).await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "data should survive re-migration");
}

/// The epic-type rebuild runs OUTSIDE sqlx's migrator (`Store::migrate` →
/// `migrate_epic_type`): a table rebuild cannot run inside sqlx's
/// per-migration transaction — `PRAGMA foreign_keys` is a no-op there, so
/// `DROP TABLE tasks` would violate the child FKs. This test exercises
/// the real upgrade path: a populated pre-epic database (sqlx migrations
/// only, old type CHECK), then `Store::open`, which performs the rebuild.
#[tokio::test]
async fn epic_rebuild_preserves_populated_database() {
    let path =
        std::env::temp_dir().join(format!("shepherd_epic_upgrade_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // Phase 1: pre-epic database — sqlx migrations only (old type CHECK),
    // populated with FK-bearing children.
    {
        let opts = SqliteConnectOptions::new()
            .filename(&path)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        seed_project_and_tasks(&pool, "p1", &["t1", "t2"]).await;
        sqlx::query(
            "INSERT INTO relations (id, type, source_task_id, target_task_id, created_at)
             VALUES ('r1', 'depends_on', 't1', 't2', '2026-09-07T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        pool.close().await;
    }

    // Phase 2: Store::open migrates — the rebuild executes against the
    // populated database. (The in-transaction version of this rebuild
    // failed here with `FOREIGN KEY constraint failed`.)
    let store = Store::open(&path).await.unwrap();
    let pool = store.pool();

    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(task_count, 2, "tasks should survive the rebuild");

    let rel_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(rel_count, 1, "relations should survive the rebuild");

    // The CHECK was widened: 'epic' is now a valid type.
    seed_project(pool, "p2").await;
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES ('e1', 'p2', 'epic', 'epic', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();

    // FK integrity verified after the rebuild.
    let fk_ok: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(fk_ok.is_empty(), "no FK violations after rebuild");

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

/// Fresh databases get the widened CHECK too (the rebuild runs on the
/// empty table right after the sqlx migrations).
#[tokio::test]
async fn epic_type_accepted_on_fresh_database() {
    let store = Store::new_in_memory().await.unwrap();
    seed_project(store.pool(), "p1").await;

    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES ('e1', 'p1', 'epic', 'epic', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .execute(store.pool())
    .await
    .unwrap();
}

// ── Test helpers ─────────────────────────────────────────────────────────

async fn seed_project(pool: &sqlx::SqlitePool, pid: &str) {
    sqlx::query(
        "INSERT INTO projects (id, name, description, review_gate, created_at, updated_at)
         VALUES (?, 'test', '', 1, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .bind(pid)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_project_and_task(pool: &sqlx::SqlitePool, pid: &str, tid: &str) {
    seed_project(pool, pid).await;

    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
         VALUES (?, ?, 'task', 'code', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
    )
    .bind(tid)
    .bind(pid)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_project_and_tasks(pool: &sqlx::SqlitePool, pid: &str, tids: &[&str]) {
    seed_project(pool, pid).await;

    for tid in tids {
        sqlx::query(
            "INSERT INTO tasks (id, project_id, title, type, status, metadata, graph_role, graph_role_explicit, attempt_count, created_at, updated_at)
             VALUES (?, ?, 'task', 'code', 'proposed', '{}', '[]', 0, 0, '2026-09-07T00:00:00Z', '2026-09-07T00:00:00Z')",
        )
        .bind(tid)
        .bind(pid)
        .execute(pool)
        .await
        .unwrap();
    }
}
