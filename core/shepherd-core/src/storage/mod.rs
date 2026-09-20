mod connect;
#[cfg(any(test, feature = "test-support"))]
pub mod rows;
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) mod rows;
mod transaction;

pub use connect::{APPLICATION_ID, EXPORT_VERSION, SCHEMA_VERSION, StoreOptions, open};
pub use transaction::TxFuture;

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::model::{ActorId, Clock};

pub struct Store {
    pool: SqlitePool,
    clock: Arc<dyn Clock>,
    codec: Arc<dyn IdempotencyCodec>,
}

impl Store {
    pub(crate) fn new(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        codec: Arc<dyn IdempotencyCodec>,
    ) -> Self {
        Self { pool, clock, codec }
    }

    // crate-internal in production: downstream crates must not bypass
    // command_transaction's reservation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    pub fn codec(&self) -> &dyn IdempotencyCodec {
        self.codec.as_ref()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("archived MVP database at {path}; refusing to open or modify it")]
    MvpDatabase { path: PathBuf },
    #[error("foreign or nonempty non-v1 database at {path}; refusing to open or modify it")]
    ForeignDatabase { path: PathBuf },
    #[error("initialization checkpoint did not reach the database header at {path}")]
    Checkpoint { path: PathBuf },
    #[error("schema identity mismatch: version {version}, export_version {export_version}")]
    SchemaMismatch {
        version: i64,
        export_version: String,
    },
    #[error("corrupt stored value: {0}")]
    Corrupt(String),
    #[error("idempotency codec: {0}")]
    Codec(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct IdempotencyAad<'a> {
    pub actor_id: &'a ActorId,
    pub key: &'a Uuid,
    pub request_hash: &'a str,
}

pub struct SealedResponse {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

pub trait ReplayKeyProvider: Send + Sync {
    fn key(&self) -> &[u8; 32];
}

pub trait IdempotencyCodec: Send + Sync {
    fn seal(
        &self,
        aad: &IdempotencyAad<'_>,
        plaintext: &[u8],
    ) -> Result<SealedResponse, StorageError>;

    fn open(
        &self,
        aad: &IdempotencyAad<'_>,
        sealed: &SealedResponse,
    ) -> Result<Vec<u8>, StorageError>;
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestKeyProvider(pub [u8; 32]);

#[cfg(any(test, feature = "test-support"))]
impl ReplayKeyProvider for TestKeyProvider {
    fn key(&self) -> &[u8; 32] {
        &self.0
    }
}

// Deliberately insecure stand-in proving the codec plumbing; AES-256-GCM replaces it in step 007.
#[cfg(any(test, feature = "test-support"))]
pub struct TestCodec {
    provider: Arc<dyn ReplayKeyProvider>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestCodec {
    pub fn new(provider: Arc<dyn ReplayKeyProvider>) -> Self {
        Self { provider }
    }

    fn keystream(&self, nonce: &[u8], data: &[u8]) -> Result<Vec<u8>, StorageError> {
        if nonce.len() != 12 {
            return Err(StorageError::Codec("nonce must be 12 bytes".into()));
        }
        let key = self.provider.key();
        Ok(data
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ key[i % key.len()] ^ nonce[i % nonce.len()])
            .collect())
    }

    fn tag(&self, nonce: &[u8], aad: &IdempotencyAad<'_>, plaintext: &[u8]) -> [u8; 8] {
        let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
        let aad_text = format!("{}\n{}\n{}", aad.actor_id, aad.key, aad.request_hash);
        for part in [
            self.provider.key().as_slice(),
            nonce,
            aad_text.as_bytes(),
            plaintext,
        ] {
            for byte in part {
                acc = (acc ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
            acc = acc.wrapping_add(1);
        }
        acc.to_be_bytes()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl IdempotencyCodec for TestCodec {
    fn seal(
        &self,
        aad: &IdempotencyAad<'_>,
        plaintext: &[u8],
    ) -> Result<SealedResponse, StorageError> {
        let nonce =
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).as_bytes()[..12].to_vec();
        let mut ciphertext = self.keystream(&nonce, plaintext)?;
        ciphertext.extend_from_slice(&self.tag(&nonce, aad, plaintext));
        Ok(SealedResponse { ciphertext, nonce })
    }

    fn open(
        &self,
        aad: &IdempotencyAad<'_>,
        sealed: &SealedResponse,
    ) -> Result<Vec<u8>, StorageError> {
        let split = sealed
            .ciphertext
            .len()
            .checked_sub(8)
            .ok_or_else(|| StorageError::Codec("sealed response too short".into()))?;
        let (body, found_tag) = sealed.ciphertext.split_at(split);
        let plaintext = self.keystream(&sealed.nonce, body)?;
        if self.tag(&sealed.nonce, aad, &plaintext) != found_tag {
            return Err(StorageError::Codec("authentication failed".into()));
        }
        Ok(plaintext)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::path::Path;
    use std::sync::Arc;

    use super::{StoreOptions, TestCodec, TestKeyProvider};
    use crate::model::TestClock;

    pub fn test_clock() -> Arc<TestClock> {
        Arc::new(TestClock::new("2026-09-14T00:00:00Z".parse().unwrap()))
    }

    pub fn store_options(dir: &Path, db_name: &str, clock: Arc<TestClock>) -> StoreOptions {
        StoreOptions {
            db_path: dir.join(db_name),
            mvp_db_path: Some(dir.join("mvp").join("shepherd.db")),
            clock,
            codec: Arc::new(TestCodec::new(Arc::new(TestKeyProvider([3; 32])))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActorId;

    fn codec() -> TestCodec {
        TestCodec::new(Arc::new(TestKeyProvider([7; 32])))
    }

    fn aad_parts() -> (ActorId, Uuid) {
        let actor = ActorId::generate("2026-09-14T00:00:00Z".parse().unwrap());
        (actor, Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)))
    }

    #[test]
    fn codec_round_trips_sealed_responses() {
        let codec = codec();
        let (actor, key) = aad_parts();
        let aad = IdempotencyAad {
            actor_id: &actor,
            key: &key,
            request_hash: "hash-1",
        };
        let sealed = codec.seal(&aad, b"response body").unwrap();
        assert_ne!(sealed.ciphertext, b"response body");
        assert_eq!(sealed.nonce.len(), 12);
        assert_eq!(codec.open(&aad, &sealed).unwrap(), b"response body");
    }

    #[test]
    fn codec_rejects_wrong_aad_and_tampering() {
        let codec = codec();
        let (actor, key) = aad_parts();
        let aad = IdempotencyAad {
            actor_id: &actor,
            key: &key,
            request_hash: "hash-1",
        };
        let sealed = codec.seal(&aad, b"response body").unwrap();

        let wrong_aad = IdempotencyAad {
            actor_id: &actor,
            key: &key,
            request_hash: "hash-2",
        };
        assert!(matches!(
            codec.open(&wrong_aad, &sealed),
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

        let short = SealedResponse {
            ciphertext: vec![1, 2, 3],
            nonce: sealed.nonce,
        };
        assert!(matches!(
            codec.open(&aad, &short),
            Err(StorageError::Codec(_))
        ));

        let empty_nonce = SealedResponse {
            ciphertext: vec![0; 16],
            nonce: Vec::new(),
        };
        assert!(matches!(
            codec.open(&aad, &empty_nonce),
            Err(StorageError::Codec(_))
        ));
    }
}
