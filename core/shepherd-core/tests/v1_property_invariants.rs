// Property invariants exercised with deterministic pseudo-random inputs: the
// sealed-response codec and the cursor verifier must never panic or accept
// tampered data, whatever bytes arrive. cargo-fuzz remains the recorded
// follow-up for coverage-guided runs; these fixed-seed properties are the
// reproducible gate.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{Actor, ActorId, ActorKind, Clock, CommandId, ProjectCreate, TestClock};
use shepherd_core::queries::ListParams;
use shepherd_core::storage::rows::insert_actor;
use shepherd_core::storage::{
    AesGcmCodec, IdempotencyCodec, SealedResponse, Store, TestKeyProvider, open, testing,
};
use uuid::Uuid;

// xorshift64*: deterministic, so a failing iteration is reproducible from the
// seed printed in the panic message.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 32) as u8
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }
}

#[test]
fn sealed_responses_round_trip_arbitrary_inputs_and_reject_tampering() {
    let mut rng = Rng::new(0x5EED_0001);
    for iteration in 0..128 {
        let key: [u8; 32] = rng.bytes(32).try_into().unwrap();
        let aad_len = (rng.next() % 64) as usize;
        let aad = rng.bytes(aad_len);
        let plaintext_len = (rng.next() % 512) as usize;
        let plaintext = rng.bytes(plaintext_len);

        let codec = AesGcmCodec::new(Arc::new(TestKeyProvider(key)));
        let sealed = codec.seal_bytes(&aad, &plaintext).unwrap();
        assert_eq!(
            codec.open_bytes(&aad, &sealed).unwrap(),
            plaintext,
            "iteration {iteration} must round trip"
        );

        // Flipping any bit anywhere in the sealed response must fail the open.
        let mut flipped = SealedResponse {
            ciphertext: sealed.ciphertext.clone(),
            nonce: sealed.nonce.clone(),
        };
        if !flipped.ciphertext.is_empty() {
            let index = (rng.next() as usize) % flipped.ciphertext.len();
            flipped.ciphertext[index] ^= 1 << (rng.next() % 8);
        }
        if !flipped.nonce.is_empty() {
            let index = (rng.next() as usize) % flipped.nonce.len();
            flipped.nonce[index] ^= 1 << (rng.next() % 8);
        }
        assert!(
            codec.open_bytes(&aad, &flipped).is_err(),
            "iteration {iteration} must reject tampered ciphertext"
        );

        // A different AAD must never authenticate the same sealed response.
        let mut other_aad = aad.clone();
        other_aad.push(0);
        assert!(
            codec.open_bytes(&other_aad, &sealed).is_err(),
            "iteration {iteration} must reject a foreign AAD"
        );
    }
}

async fn store_with_owner() -> (tempfile::TempDir, Store, Actor) {
    let dir = tempfile::tempdir().unwrap();
    let clock: Arc<TestClock> = testing::test_clock();
    let store = open(testing::store_options(
        dir.path(),
        "shepherd.db",
        clock.clone(),
    ))
    .await
    .unwrap();
    let owner = Actor {
        id: ActorId::generate(clock.now()),
        kind: ActorKind::Human,
        label: "owner".to_string(),
        revoked: false,
        created_at: clock.now(),
    };
    let inserted = owner.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
        .await
        .unwrap();
    store
        .create_project(
            CommandContext {
                actor: owner.clone(),
                command_id: CommandId::generate(clock.now()),
                idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
                expected_revision: None,
                now: clock.now(),
            },
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (dir, store, owner)
}

#[tokio::test]
async fn arbitrary_cursor_strings_are_rejected_without_panicking() {
    let (_dir, store, _owner) = store_with_owner().await;
    let mut rng = Rng::new(0x5EED_0002);
    for iteration in 0..256 {
        let len = (rng.next() % 128) as usize;
        let raw = rng.bytes(len);
        // Both raw-lossy and base64url-shaped inputs reach the verifier.
        for candidate in [
            String::from_utf8_lossy(&raw).into_owned(),
            URL_SAFE_NO_PAD.encode(&raw),
        ] {
            let err = store
                .list_projects(&ListParams {
                    limit: None,
                    cursor: Some(candidate.clone()),
                    include_archived: false,
                })
                .await
                .expect_err("arbitrary cursors must never list");
            assert!(
                matches!(err, DomainError::InvalidCursor(_)),
                "iteration {iteration} must classify as an invalid cursor, got {err:?}"
            );
        }
    }
}
