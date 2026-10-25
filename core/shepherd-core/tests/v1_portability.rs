use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use shepherd_core::commands::{CommandContext, IdentityPaths, Replay};
use shepherd_core::model::{
    Actor, ClaimInput, ClaimPhase, CommandId, EpicCreate, GoalCreate, ProjectCreate, ProjectId,
    Task, TaskCreate, TestClock,
};
use shepherd_core::storage::maintenance::BackupManifest;
use shepherd_core::storage::{
    FileReplayKeyProvider, StorageError, Store, maintenance, open, testing,
};

struct Fixture {
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
}

struct Seeded {
    owner: Actor,
    project: ProjectId,
    task: Task,
}

impl Fixture {
    fn data_dir(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn paths(&self) -> IdentityPaths {
        IdentityPaths::new(self.dir.path())
    }

    fn backup_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("backup")
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    Fixture { dir, clock }
}

fn ctx(store: &Store, actor: &Actor) -> CommandContext {
    let now = store.clock().now();
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key: uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        expected_revision: None,
        now,
    }
}

/// Open a store, seed an owner, a project and a claimable task, provision
/// credential files, then close the pool.
async fn seed_data(f: &Fixture) -> Seeded {
    // Provision a replay-key file so backup can copy it.
    FileReplayKeyProvider::provision(&f.paths().replay_key()).unwrap();

    let store = open(testing::store_options(
        f.data_dir(),
        "shepherd.db",
        f.clock.clone(),
    ))
    .await
    .unwrap();

    let owner = store.ensure_owner(&f.paths()).await.unwrap().actor;
    let project = store
        .create_project(
            ctx(&store, &owner),
            ProjectCreate {
                name: "backup-test".into(),
                description: Some("project for portability tests".into()),
                settings: None,
            },
        )
        .await
        .unwrap()
        .value;

    f.clock.advance(chrono::TimeDelta::milliseconds(2));
    let goal = store
        .create_goal(
            ctx(&store, &owner),
            project.id,
            GoalCreate {
                title: "portability".into(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    f.clock.advance(chrono::TimeDelta::milliseconds(2));
    let epic = store
        .create_epic(
            ctx(&store, &owner),
            project.id,
            goal.id,
            EpicCreate {
                title: "seed epic".into(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    f.clock.advance(chrono::TimeDelta::milliseconds(2));
    let task = store
        .create_task(
            ctx(&store, &owner),
            project.id,
            epic.id,
            TaskCreate {
                title: "seed task".into(),
                type_key: "code".to_string(),
                planning_required: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;

    store.pool().close().await;
    Seeded {
        owner,
        project: project.id,
        task,
    }
}

// ---------------------------------------------------------------------------
// Backup tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backup_creates_directory_and_files_with_correct_modes() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    let dir_mode = std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "backup directory should be 0700");

    for name in ["shepherd.db", "owner-token", "replay-key", "manifest.json"] {
        let path = backup.join(name);
        assert!(path.exists(), "{name} should exist in backup");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{name} should be 0600");
    }
}

#[tokio::test]
async fn backup_manifest_contains_valid_checksums_no_secrets() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    let manifest_bytes = std::fs::read(backup.join("manifest.json")).unwrap();
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).unwrap();

    assert_eq!(manifest.version, "1");
    assert_eq!(manifest.files.len(), 3);

    // Verify each digest matches the actual file.
    for entry in &manifest.files {
        let file_bytes = std::fs::read(backup.join(&entry.name)).unwrap();
        let actual = sha256_hex(&file_bytes);
        assert_eq!(actual, entry.sha256, "checksum mismatch for {}", entry.name);
    }

    // Manifest must not contain token contents.
    let manifest_text = String::from_utf8_lossy(&manifest_bytes);
    let owner_token = std::fs::read_to_string(f.data_dir().join("owner-token")).unwrap();
    let replay_key_bytes = std::fs::read(f.data_dir().join("replay-key")).unwrap();
    assert!(
        !manifest_text.contains(owner_token.trim()),
        "manifest must not contain owner-token content"
    );
    // Replay key is binary; check its base64 doesn't appear either.
    let replay_b64 = base64_encode(&replay_key_bytes);
    assert!(
        !manifest_text.contains(&replay_b64),
        "manifest must not contain replay-key content"
    );
}

#[tokio::test]
async fn backup_refuses_existing_output_directory() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    std::fs::create_dir_all(&backup).unwrap();

    let err = maintenance::backup(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::InvalidManifest(ref msg) if msg.contains("already exists")),
        "expected InvalidManifest, got: {err}"
    );
}

#[tokio::test]
async fn backup_refuses_when_daemon_lock_held() {
    let f = fixture().await;
    seed_data(&f).await;

    let _lock =
        shepherd_core::storage::DaemonLock::acquire(&f.data_dir().join("daemon.lock")).unwrap();

    let backup = f.backup_dir();
    let err = maintenance::backup(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::DaemonLocked { .. }),
        "expected DaemonLocked, got: {err}"
    );
}

#[tokio::test]
async fn backup_produces_no_wal_sidecars() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    assert!(
        !backup.join("shepherd.db-wal").exists(),
        "backup must not contain WAL sidecar"
    );
    assert!(
        !backup.join("shepherd.db-shm").exists(),
        "backup must not contain SHM sidecar"
    );
}

// ---------------------------------------------------------------------------
// Restore tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restore_preserves_completed_state_and_credentials() {
    let f = fixture().await;
    let seeded = seed_data(&f).await;

    // Seed a replayable command: a plan claim with a retained idempotency
    // key that must still replay after the backup is restored.
    let store = open(testing::store_options(
        f.data_dir(),
        "shepherd.db",
        f.clock.clone(),
    ))
    .await
    .unwrap();
    let claim_key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let claim_ctx = |store: &Store, actor: &Actor, key: uuid::Uuid| {
        let now = store.clock().now();
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(now),
            idempotency_key: key,
            expected_revision: Some(seeded.task.revision.value()),
            now,
        }
    };
    let grant = store
        .claim_task(
            claim_ctx(&store, &seeded.owner, claim_key),
            seeded.project,
            seeded.task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: Some(600),
            },
            "portability-claim-hash",
        )
        .await
        .unwrap();
    let grant = match grant {
        Replay::Fresh(grant) => grant,
        Replay::Replayed(_) => panic!("first claim must be fresh"),
    };
    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    store.pool().close().await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    let original_token = std::fs::read_to_string(f.data_dir().join("owner-token")).unwrap();
    let original_key = std::fs::read(f.data_dir().join("replay-key")).unwrap();

    // Corrupt the live database to prove restore replaces it.
    std::fs::write(f.data_dir().join("shepherd.db"), b"corrupted").unwrap();

    let recovery = maintenance::restore(f.data_dir(), &backup).await.unwrap();

    // Credential files match the originals.
    let restored_token = std::fs::read_to_string(f.data_dir().join("owner-token")).unwrap();
    let restored_key = std::fs::read(f.data_dir().join("replay-key")).unwrap();
    assert_eq!(restored_token, original_token);
    assert_eq!(restored_key, original_key);

    // The store opens on the restored data; ensure_owner must report the
    // persisted owner rather than bootstrapping a fresh identity.
    let store = open(testing::store_options(
        f.data_dir(),
        "shepherd.db",
        f.clock.clone(),
    ))
    .await
    .unwrap();
    let bootstrapped = store.ensure_owner(&f.paths()).await.unwrap();
    assert_eq!(bootstrapped.actor.id, seeded.owner.id);
    assert!(
        !bootstrapped.created,
        "restore must not bootstrap a new owner over the persisted one"
    );

    // The seeded project survived the round trip.
    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = ?1")
        .bind(seeded.project.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(project_name, "backup-test");

    // Retrying the claim command replays the original response: no new
    // claim row and no duplicate event.
    let retry = store
        .claim_task(
            claim_ctx(&store, &seeded.owner, claim_key),
            seeded.project,
            seeded.task.id,
            ClaimInput {
                phase: ClaimPhase::Plan,
                submission_id: None,
                ttl_seconds: Some(600),
            },
            "portability-claim-hash",
        )
        .await
        .unwrap();
    let replayed = match retry {
        Replay::Replayed(grant) => grant,
        Replay::Fresh(_) => panic!("retry after restore must replay, not create a new claim"),
    };
    assert_eq!(replayed.claim.id, grant.claim.id);
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM claims")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(claims, 1, "replay must not add a duplicate claim");
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(events_after, events_before, "replay must not add events");
    store.pool().close().await;

    // Recovery directory exists with the corrupted file.
    let recovery_path = f.data_dir().join(&recovery);
    assert!(recovery_path.exists(), "recovery directory should exist");
    let recovered_db = std::fs::read(recovery_path.join("shepherd.db")).unwrap();
    assert_eq!(recovered_db, b"corrupted");
}

#[tokio::test]
async fn restore_rejects_wrong_checksum() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Tamper with the database file after backup.
    let db_path = backup.join("shepherd.db");
    let mut bytes = std::fs::read(&db_path).unwrap();
    if let Some(b) = bytes.last_mut() {
        *b ^= 0xFF;
    }
    std::fs::write(&db_path, &bytes).unwrap();

    let original_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got: {err}"
    );

    // Live data must be untouched.
    let current_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();
    assert_eq!(
        current_db, original_db,
        "live database must not be modified"
    );
}

#[tokio::test]
async fn restore_rejects_group_readable_credential() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Make the backup's owner-token group-readable.
    std::fs::set_permissions(
        backup.join("owner-token"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::CredentialFile(..)),
        "expected CredentialFile, got: {err}"
    );
}

#[tokio::test]
async fn restore_rejects_incompatible_schema() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Modify the backup DB's schema_meta to an incompatible version.
    let db_path = backup.join("shepherd.db");
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&db_path);
    let mut conn = sqlx::ConnectOptions::connect(&opts).await.unwrap();
    sqlx::query("UPDATE schema_meta SET version = 99")
        .execute(&mut conn)
        .await
        .unwrap();
    use sqlx::Connection;
    conn.close().await.unwrap();

    // Re-hash the manifest so checksum verification passes.
    rehash_manifest(&backup);

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::SchemaMismatch { .. }),
        "expected SchemaMismatch, got: {err}"
    );
}

#[tokio::test]
async fn restore_rejects_failed_integrity_check() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Corrupt the backup DB by zeroing an entire data page (page 2+) so
    // SQLite's integrity_check or schema_meta query fails.
    let db_path = backup.join("shepherd.db");
    let mut bytes = std::fs::read(&db_path).unwrap();
    let page_size = 4096;
    if bytes.len() > page_size * 2 {
        // Wipe page 2 (the first non-header page containing table data).
        for b in &mut bytes[page_size..page_size * 2] {
            *b = 0x00;
        }
    }
    std::fs::write(&db_path, &bytes).unwrap();

    // Re-hash the manifest.
    rehash_manifest(&backup);

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    // Corruption surfaces as integrity failure, schema query error, or malformed image.
    let msg = err.to_string();
    assert!(
        msg.contains("integrity")
            || msg.contains("malformed")
            || msg.contains("schema")
            || matches!(err, StorageError::Sqlx(..)),
        "expected integrity/schema failure, got: {err}"
    );
}

#[tokio::test]
async fn restore_preserves_original_in_recovery_directory() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    let recovery = maintenance::restore(f.data_dir(), &backup).await.unwrap();
    let recovery_path = f.data_dir().join(&recovery);

    assert!(recovery_path.exists());
    let mode = std::fs::metadata(&recovery_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "recovery directory should be 0700");

    assert!(recovery_path.join("shepherd.db").exists());
    assert!(recovery_path.join("owner-token").exists());
    assert!(recovery_path.join("replay-key").exists());
}

#[tokio::test]
async fn restore_rejects_manifest_without_database() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Strip the database entry: without validation the live database would
    // move to recovery, never be replaced, and the next boot would provision
    // an empty database.
    let manifest_path = backup.join("manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest.files.retain(|entry| entry.name != "shepherd.db");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let original_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::InvalidManifest(ref msg) if msg.contains("shepherd.db")),
        "expected InvalidManifest, got: {err}"
    );

    // Live data must be untouched.
    let current_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();
    assert_eq!(
        current_db, original_db,
        "live database must not be modified"
    );
}

#[tokio::test]
async fn restore_rejects_duplicate_manifest_entries() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Three entries with a repeated owner-token: a count-based check would
    // pass this, only name-based duplicate detection rejects it.
    let manifest_path = backup.join("manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest.files.retain(|entry| entry.name != "replay-key");
    let credential = manifest
        .files
        .iter()
        .find(|entry| entry.name == "owner-token")
        .cloned()
        .unwrap();
    manifest.files.push(credential);
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let original_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::InvalidManifest(ref msg) if msg.contains("duplicate")),
        "expected InvalidManifest, got: {err}"
    );

    let current_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();
    assert_eq!(
        current_db, original_db,
        "live database must not be modified"
    );
}

#[tokio::test]
async fn restore_rejects_manifest_mismatched_with_backup_contents() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    // Drop the owner-token entry while the file stays in the backup: a
    // silent credential drop would bootstrap a new owner on next boot.
    let manifest_path = backup.join("manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest.files.retain(|entry| entry.name != "owner-token");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let original_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();
    let original_token = std::fs::read_to_string(f.data_dir().join("owner-token")).unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::InvalidManifest(ref msg) if msg.contains("owner-token")),
        "expected InvalidManifest, got: {err}"
    );

    let current_db = std::fs::read(f.data_dir().join("shepherd.db")).unwrap();
    assert_eq!(current_db, original_db);
    let current_token = std::fs::read_to_string(f.data_dir().join("owner-token")).unwrap();
    assert_eq!(current_token, original_token);
}

#[tokio::test]
async fn restore_refuses_when_daemon_lock_held() {
    let f = fixture().await;
    seed_data(&f).await;

    let backup = f.backup_dir();
    maintenance::backup(f.data_dir(), &backup).await.unwrap();

    let _lock =
        shepherd_core::storage::DaemonLock::acquire(&f.data_dir().join("daemon.lock")).unwrap();

    let err = maintenance::restore(f.data_dir(), &backup)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StorageError::DaemonLocked { .. }),
        "expected DaemonLocked, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Re-compute the manifest checksums after modifying a backup file.
fn rehash_manifest(backup: &std::path::Path) {
    let manifest_path = backup.join("manifest.json");
    let raw = std::fs::read(&manifest_path).unwrap();
    let mut manifest: BackupManifest = serde_json::from_slice(&raw).unwrap();
    for entry in &mut manifest.files {
        let file_bytes = std::fs::read(backup.join(&entry.name)).unwrap();
        entry.sha256 = sha256_hex(&file_bytes);
    }
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    std::fs::write(&manifest_path, json.as_bytes()).unwrap();
}
