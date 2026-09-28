use std::fs::{self, File, OpenOptions, Permissions};
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use fs2::FileExt;
use sqlx::{Sqlite, Transaction};

use super::{IdempotencyCodec, ReplayKeyProvider, SealedResponse, StorageError, Store};

// Boxed, not AsyncFnOnce: spawned callers trip rustc's AsyncFnOnce "not general enough" limit.
pub type TxFuture<'t, T> = Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 't>>;

impl Store {
    // Write reservation before reads (plan/05): the no-op UPDATE upgrades BEGIN to a write lock.
    pub(crate) async fn begin_command(&self) -> Result<Transaction<'static, Sqlite>, StorageError> {
        let mut tx = self.pool().begin().await?;
        let reserved = sqlx::query("UPDATE command_lock SET value=value WHERE id=1")
            .execute(&mut *tx)
            .await?;
        if reserved.rows_affected() != 1 {
            tx.rollback().await.ok();
            return Err(StorageError::Corrupt(
                "command_lock row missing; write serialization unavailable".into(),
            ));
        }
        Ok(tx)
    }

    pub async fn command_transaction<T, F>(&self, command: F) -> Result<T, StorageError>
    where
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> TxFuture<'t, T>,
    {
        let mut tx = self.begin_command().await?;
        match command(&mut tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(err) => {
                tx.rollback().await.ok();
                Err(err)
            }
        }
    }
}

/// Production replay codec (plan/12): AES-256-GCM, random 96-bit nonce,
/// caller-supplied authenticated associated data.
pub struct AesGcmCodec {
    provider: Arc<dyn ReplayKeyProvider>,
}

impl AesGcmCodec {
    pub fn new(provider: Arc<dyn ReplayKeyProvider>) -> Self {
        Self { provider }
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&(*self.provider.key()).into())
    }
}

impl IdempotencyCodec for AesGcmCodec {
    fn seal_bytes(&self, aad: &[u8], plaintext: &[u8]) -> Result<SealedResponse, StorageError> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).expect("system RNG is available");
        let ciphertext = self
            .cipher()
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| StorageError::Codec("seal failed".into()))?;
        Ok(SealedResponse {
            ciphertext,
            nonce: nonce.to_vec(),
        })
    }

    fn open_bytes(&self, aad: &[u8], sealed: &SealedResponse) -> Result<Vec<u8>, StorageError> {
        let nonce: [u8; 12] = sealed
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| StorageError::Codec("nonce must be 12 bytes".into()))?;
        self.cipher()
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: &sealed.ciphertext,
                    aad,
                },
            )
            .map_err(|_| StorageError::Codec("authentication failed".into()))
    }
}

/// Loads `<data-dir>/replay-key`. Missing key on an existing install is the
/// caller's diagnostic-only signal; never regenerate one here (plan/12).
pub struct FileReplayKeyProvider {
    key: [u8; 32],
}

impl FileReplayKeyProvider {
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        verify_secret_file_mode(path)?;
        let bytes = fs::read(path)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| {
            StorageError::CredentialFile(format!(
                "replay key at {} must be exactly 32 bytes; restore it from a \
                 backup (or delete the whole data directory to start fresh)",
                path.display()
            ))
        })?;
        Ok(Self { key })
    }

    /// Fresh installs only: the caller must have proven the database is absent.
    pub fn provision(path: &Path) -> Result<Self, StorageError> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).expect("system RNG is available");
        write_secret_file(path, &key)?;
        Ok(Self { key })
    }
}

impl ReplayKeyProvider for FileReplayKeyProvider {
    fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

/// Writes 0600 under a 0700 parent, replacing any previous contents (plan/12).
pub fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| {
        StorageError::CredentialFile(format!("{} has no parent directory", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    // Same-directory temp file + rename: a crash mid-write can never leave a
    // truncated secret behind, only the old file or the complete new one.
    let staging = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staging)?;
    // mode() applies only at creation; a pre-existing file keeps its old bits.
    file.set_permissions(Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&staging, path)?;
    Ok(())
}

/// Backup-restore parity with plan/13: reject secrets readable by group/others.
pub fn verify_secret_file_mode(path: &Path) -> Result<(), StorageError> {
    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(StorageError::CredentialFile(format!(
            "{} is readable by group or others (mode {:o}); expected 0600",
            path.display(),
            mode & 0o777
        )));
    }
    Ok(())
}

/// Advisory cross-process lock on `<data-dir>/daemon.lock`; held until drop
/// (plan/05: reject second daemon ownership).
pub struct DaemonLock {
    _file: File,
}

impl DaemonLock {
    pub fn acquire(path: &Path) -> Result<Self, StorageError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().map_err(|err| {
            if err.kind() == fs2::lock_contended_error().kind() {
                StorageError::DaemonLocked {
                    path: path.to_path_buf(),
                }
            } else {
                StorageError::Io(err)
            }
        })?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Actor, ActorId, ActorKind};
    use crate::storage::IdempotencyAad;
    use crate::storage::open;
    use crate::storage::rows::{get_actor, insert_actor};
    use crate::storage::testing::{store_options, test_clock};

    async fn test_store(dir: &tempfile::TempDir) -> Store {
        open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap()
    }

    fn actor(store: &Store, label: &str) -> Actor {
        let now = store.clock().now();
        Actor {
            id: ActorId::generate(now),
            kind: ActorKind::Agent,
            label: label.to_string(),
            revoked: false,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn committed_transaction_persists_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        let actor = actor(&store, "worker");
        let inserted = actor.clone();
        store
            .command_transaction(|tx| {
                Box::pin(async move {
                    insert_actor(tx, &inserted).await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
        assert_eq!(
            get_actor(store.pool(), &actor.id).await.unwrap(),
            Some(actor)
        );
    }

    #[tokio::test]
    async fn failing_transaction_rolls_back_all_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        let actor = actor(&store, "worker");
        let inserted = actor.clone();
        let err = store
            .command_transaction(|tx| {
                Box::pin(async move {
                    insert_actor(tx, &inserted).await?;
                    Err::<(), _>(StorageError::Corrupt("injected failure".into()))
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Corrupt(_)));
        assert_eq!(get_actor(store.pool(), &actor.id).await.unwrap(), None);
    }

    #[test]
    fn gcm_codec_round_trips_and_rejects_foreign_actor_aad() {
        let dir = tempfile::tempdir().unwrap();
        let provider =
            Arc::new(FileReplayKeyProvider::provision(&dir.path().join("replay-key")).unwrap());
        let codec = AesGcmCodec::new(provider);
        let now = "2026-09-14T00:00:00Z".parse().unwrap();
        let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let owner = ActorId::generate(now);
        let aad = IdempotencyAad {
            actor_id: &owner,
            key: &key,
            request_hash: "hash-1",
        };
        let sealed = codec.seal(&aad, b"claim grant").unwrap();
        assert_ne!(sealed.ciphertext, b"claim grant");
        assert_eq!(sealed.nonce.len(), 12);
        assert_eq!(codec.open(&aad, &sealed).unwrap(), b"claim grant");

        let other = ActorId::generate(now);
        let foreign = IdempotencyAad {
            actor_id: &other,
            key: &key,
            request_hash: "hash-1",
        };
        assert!(matches!(
            codec.open(&foreign, &sealed),
            Err(StorageError::Codec(_))
        ));

        let mut tampered = SealedResponse {
            ciphertext: sealed.ciphertext.clone(),
            nonce: sealed.nonce.clone(),
        };
        tampered.ciphertext[0] ^= 0xff;
        assert!(matches!(
            codec.open(&aad, &tampered),
            Err(StorageError::Codec(_))
        ));

        let bad_nonce = SealedResponse {
            ciphertext: sealed.ciphertext,
            nonce: vec![0; 4],
        };
        assert!(matches!(
            codec.open(&aad, &bad_nonce),
            Err(StorageError::Codec(_))
        ));
    }

    #[test]
    fn replay_key_files_have_0600_and_dir_0700() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets").join("replay-key");
        let provisioned = FileReplayKeyProvider::provision(&path).unwrap();
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let loaded = FileReplayKeyProvider::load(&path).unwrap();
        assert_eq!(loaded.key(), provisioned.key());
    }

    #[test]
    fn world_readable_replay_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay-key");
        FileReplayKeyProvider::provision(&path).unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            FileReplayKeyProvider::load(&path),
            Err(StorageError::CredentialFile(_))
        ));
    }

    #[test]
    fn wrong_size_replay_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay-key");
        write_secret_file(&path, b"short").unwrap();
        assert!(matches!(
            FileReplayKeyProvider::load(&path),
            Err(StorageError::CredentialFile(_))
        ));
    }

    #[test]
    fn write_secret_file_replaces_content_and_restores_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner-token");
        write_secret_file(&path, b"first").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        write_secret_file(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn second_daemon_is_refused_by_advisory_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.lock");
        let held = DaemonLock::acquire(&path).unwrap();
        assert!(matches!(
            DaemonLock::acquire(&path),
            Err(StorageError::DaemonLocked { .. })
        ));
        drop(held);
        DaemonLock::acquire(&path).unwrap();
    }

    #[tokio::test]
    async fn missing_command_lock_row_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        sqlx::query("DELETE FROM command_lock")
            .execute(store.pool())
            .await
            .unwrap();
        let err = store
            .command_transaction(|_tx| Box::pin(async move { Ok(()) }))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Corrupt(_)));
    }
}
