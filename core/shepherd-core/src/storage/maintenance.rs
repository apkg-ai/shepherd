use std::fs::{self, File, Permissions};
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Row};

use super::transaction::{DaemonLock, verify_secret_file_mode, write_secret_file};
use super::{APPLICATION_ID, EXPORT_VERSION, SCHEMA_VERSION, StorageError};

const MANIFEST_VERSION: &str = "1";
const DB_FILE: &str = "shepherd.db";
const OWNER_TOKEN_FILE: &str = "owner-token";
const REPLAY_KEY_FILE: &str = "replay-key";
/// Only these names are accepted in a manifest; anything else is rejected
/// before any file I/O to prevent path traversal.
const ALLOWED_FILES: [&str; 3] = [DB_FILE, OWNER_TOKEN_FILE, REPLAY_KEY_FILE];

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ManifestFile {
    pub name: String,
    pub sha256: String,
}

fn sha256_file(path: &Path) -> Result<String, StorageError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Creates a directory with mode 0700 atomically (no TOCTOU window).
fn create_dir_restricted(path: &Path) -> Result<(), StorageError> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

fn write_file_restricted(path: &Path, content: &[u8]) -> Result<(), StorageError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

/// Create an offline backup of the data directory.
///
/// The daemon must not be running (the lock file is acquired for the
/// duration). `output` must not already exist. On failure after directory
/// creation the partial output is removed.
pub async fn backup(data_dir: &Path, output: &Path) -> Result<(), StorageError> {
    let lock_path = data_dir.join("daemon.lock");
    let _lock = DaemonLock::acquire(&lock_path)?;

    if output.exists() {
        return Err(StorageError::InvalidManifest(format!(
            "backup directory {} already exists; choose a new location",
            output.display()
        )));
    }

    create_dir_restricted(output)?;

    let result = backup_inner(data_dir, output).await;
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

async fn backup_inner(data_dir: &Path, output: &Path) -> Result<(), StorageError> {
    let src_db = data_dir.join(DB_FILE);
    if !src_db.exists() {
        return Err(StorageError::InvalidManifest(format!(
            "no database found at {}",
            src_db.display()
        )));
    }

    // VACUUM INTO produces a single-file consistent snapshot (no WAL
    // sidecars). The file inherits the process umask, but the parent
    // directory is already 0700 so no other user can reach it.
    let dst_db = output.join(DB_FILE);
    let dst_str = dst_db
        .to_str()
        .ok_or_else(|| StorageError::InvalidManifest("backup path is not valid UTF-8".into()))?;
    let mut conn = SqliteConnectOptions::new()
        .filename(&src_db)
        .read_only(true)
        .connect()
        .await?;
    sqlx::query("VACUUM INTO ?1")
        .bind(dst_str)
        .execute(&mut conn)
        .await?;
    conn.close().await.ok();
    fs::set_permissions(&dst_db, Permissions::from_mode(0o600))?;

    // Copy credential files (atomic 0600 writes).
    for name in [OWNER_TOKEN_FILE, REPLAY_KEY_FILE] {
        let src = data_dir.join(name);
        if src.exists() {
            let bytes = fs::read(&src)?;
            write_secret_file(&output.join(name), &bytes)?;
        }
    }

    // Manifest: SHA-256 digests of the output files, never token contents.
    let mut files = Vec::new();
    for name in ALLOWED_FILES {
        let path = output.join(name);
        if path.exists() {
            files.push(ManifestFile {
                name: name.to_string(),
                sha256: sha256_file(&path)?,
            });
        }
    }

    let manifest = BackupManifest {
        version: MANIFEST_VERSION.to_string(),
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        files,
    };
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| StorageError::InvalidManifest(e.to_string()))?;
    write_file_restricted(&output.join("manifest.json"), json.as_bytes())?;

    Ok(())
}

/// Restore from an offline backup directory.
///
/// The daemon must not be running. Verifies checksums, file modes, schema
/// compatibility and SQLite integrity before touching the live data
/// directory. The current live files are preserved in a timestamped
/// recovery directory.
pub async fn restore(data_dir: &Path, from: &Path) -> Result<String, StorageError> {
    let lock_path = data_dir.join("daemon.lock");
    let _lock = DaemonLock::acquire(&lock_path)?;

    let manifest = read_manifest(from)?;
    verify_checksums(from, &manifest)?;
    verify_credential_modes(from)?;
    verify_backup_db(from).await?;

    let recovery_dir = stage_and_swap(data_dir, from, &manifest)?;
    Ok(recovery_dir)
}

fn read_manifest(from: &Path) -> Result<BackupManifest, StorageError> {
    let manifest_path = from.join("manifest.json");
    let bytes = fs::read(&manifest_path).map_err(|e| {
        StorageError::InvalidManifest(format!("cannot read {}: {e}", manifest_path.display()))
    })?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|e| StorageError::InvalidManifest(e.to_string()))?;
    if manifest.version != MANIFEST_VERSION {
        return Err(StorageError::InvalidManifest(format!(
            "unsupported manifest version: {}",
            manifest.version
        )));
    }
    validate_manifest_names(&manifest)?;
    Ok(manifest)
}

/// Reject manifest entries whose names are not in the allowlist, duplicate
/// entries, and manifests without the database.
fn validate_manifest_names(manifest: &BackupManifest) -> Result<(), StorageError> {
    for entry in &manifest.files {
        if !ALLOWED_FILES.contains(&entry.name.as_str()) {
            return Err(StorageError::InvalidManifest(format!(
                "unexpected file in manifest: {:?}",
                entry.name
            )));
        }
    }
    // Without a database entry the live database would move to the recovery
    // directory and never be replaced: the next boot provisions an empty
    // database, silently wiping all data.
    if manifest
        .files
        .iter()
        .filter(|entry| entry.name == DB_FILE)
        .count()
        != 1
    {
        return Err(StorageError::InvalidManifest(
            "manifest must contain exactly one shepherd.db entry".into(),
        ));
    }
    if manifest.files.len() > ALLOWED_FILES.len() {
        return Err(StorageError::InvalidManifest(
            "manifest contains duplicate file entries".into(),
        ));
    }
    Ok(())
}

fn verify_file_checksum(path: &Path, entry: &ManifestFile) -> Result<(), StorageError> {
    let actual = sha256_file(path)?;
    if actual != entry.sha256 {
        return Err(StorageError::ChecksumMismatch {
            file: entry.name.clone(),
            expected: entry.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

fn verify_checksums(from: &Path, manifest: &BackupManifest) -> Result<(), StorageError> {
    for entry in &manifest.files {
        verify_file_checksum(&from.join(&entry.name), entry)?;
    }
    Ok(())
}

fn verify_credential_modes(from: &Path) -> Result<(), StorageError> {
    for name in [OWNER_TOKEN_FILE, REPLAY_KEY_FILE] {
        let path = from.join(name);
        if path.exists() {
            verify_secret_file_mode(&path)?;
        }
    }
    Ok(())
}

async fn verify_backup_db(from: &Path) -> Result<(), StorageError> {
    let db_path = from.join(DB_FILE);
    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .read_only(true)
        .connect()
        .await?;

    // Schema identity check.
    let id: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&mut conn)
        .await?;
    let row = sqlx::query("SELECT version, export_version FROM schema_meta")
        .fetch_one(&mut conn)
        .await?;
    let version: i64 = row.get("version");
    let export_version: String = row.get("export_version");

    if id != APPLICATION_ID || version != SCHEMA_VERSION || export_version != EXPORT_VERSION {
        conn.close().await.ok();
        return Err(StorageError::SchemaMismatch {
            version,
            export_version,
        });
    }

    // Integrity check.
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut conn)
        .await?;
    conn.close().await.ok();
    if result != "ok" {
        return Err(StorageError::InvalidManifest(format!(
            "backup database failed integrity check: {result}"
        )));
    }

    Ok(())
}

fn stage_and_swap(
    data_dir: &Path,
    from: &Path,
    manifest: &BackupManifest,
) -> Result<String, StorageError> {
    // Staging directory for atomic-ish swap.
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).expect("system RNG is available");
    let staging = data_dir.join(format!(".restore-staging-{}", u64::from_be_bytes(nonce)));
    create_dir_restricted(&staging)?;

    let staged = (|| -> Result<(), StorageError> {
        for entry in &manifest.files {
            let src = from.join(&entry.name);
            let dst = staging.join(&entry.name);
            if entry.name == DB_FILE {
                // Stream the database via fs::copy to avoid loading it all.
                fs::copy(&src, &dst)?;
                fs::set_permissions(&dst, Permissions::from_mode(0o600))?;
                // fs::copy does not flush; without a sync a power loss right
                // after restore could leave a truncated database behind.
                File::open(&dst)?.sync_all()?;
            } else {
                let bytes = fs::read(&src)?;
                write_secret_file(&dst, &bytes)?;
            }
        }
        Ok(())
    })();

    if let Err(e) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // Re-verify the staged bytes: files could change in the source between
    // the pre-swap verification and this copy.
    let verified = verify_checksums(&staging, manifest);
    if let Err(e) = verified {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // Move current live files into a timestamped recovery directory.
    let ts = Utc::now().format("%Y%m%d-%H%M%S");
    let recovery_name = format!("recovery-{ts}");
    let recovery = data_dir.join(&recovery_name);
    if let Err(e) = create_dir_restricted(&recovery) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // Track how far we got so a failed rename can attempt rollback.
    let mut moved_to_recovery: Vec<String> = Vec::new();

    let move_result = (|| -> Result<(), StorageError> {
        for name in [DB_FILE, OWNER_TOKEN_FILE, REPLAY_KEY_FILE] {
            let live = data_dir.join(name);
            if live.exists() {
                fs::rename(&live, recovery.join(name))?;
                moved_to_recovery.push(name.to_string());
            }
        }
        // WAL sidecars must move with the database to avoid corruption.
        for sidecar in ["shepherd.db-wal", "shepherd.db-shm"] {
            let live = data_dir.join(sidecar);
            if live.exists() {
                fs::rename(&live, recovery.join(sidecar))?;
                moved_to_recovery.push(sidecar.to_string());
            }
        }

        // Swap staged files into the data directory.
        for entry in &manifest.files {
            fs::rename(staging.join(&entry.name), data_dir.join(&entry.name))?;
        }
        Ok(())
    })();

    if let Err(e) = move_result {
        // Best-effort rollback: move recovered files back to data_dir.
        for name in &moved_to_recovery {
            let _ = fs::rename(recovery.join(name), data_dir.join(name));
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    let _ = fs::remove_dir(&staging);

    // Flush directory entries so the swap survives a power loss.
    let _ = sync_dir(data_dir);

    Ok(recovery_name)
}
