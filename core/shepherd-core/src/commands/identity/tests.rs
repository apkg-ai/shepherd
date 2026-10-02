// Identity lifecycle tests, kept in one module because the lifecycle
// crosses the bootstrap/agents/sessions seams.
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use uuid::Uuid;

use super::*;
use crate::commands::{CommandContext, Replay};
use crate::error::DomainError;
use crate::model::{
    Ack, Actor, ActorId, ActorKind, AgentCreate, AgentTokenGrant, Clock, CommandId, SecretString,
    TestClock, generate_token,
};
use crate::queries::ListParams;
use crate::storage::rows::format_ts;
use crate::storage::testing::{store_options, test_clock};
use crate::storage::{StorageError, Store, open, write_secret_file};

struct Fixture {
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
}

impl Fixture {
    fn paths(&self) -> IdentityPaths {
        IdentityPaths::new(self.dir.path())
    }

    fn ctx(&self, actor: &Actor) -> CommandContext {
        let now = self.clock.now();
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(now),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision: None,
            now,
        }
    }

    async fn owner(&self) -> Actor {
        self.store.ensure_owner(&self.paths()).await.unwrap().actor
    }

    async fn owner_token(&self) -> SecretString {
        SecretString::new(std::fs::read_to_string(self.paths().owner_token()).unwrap())
    }

    async fn agent(&self, owner: &Actor, label: &str) -> AgentTokenGrant {
        self.store
            .create_agent(
                self.ctx(owner),
                AgentCreate {
                    label: label.to_string(),
                },
            )
            .await
            .unwrap()
            .value
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(self.store.pool())
            .await
            .unwrap()
    }

    async fn audit_actions(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT action FROM security_audit ORDER BY id")
            .fetch_all(self.store.pool())
            .await
            .unwrap()
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = test_clock();
    let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
        .await
        .unwrap();
    Fixture { dir, clock, store }
}

fn assert_locked_out(err: DomainError) {
    match err {
        DomainError::Storage(StorageError::CredentialFile(message)) => {
            assert!(message.contains("--reissue-owner-token"), "{message}");
        }
        other => panic!("expected credential-file error, got {other:?}"),
    }
}

#[tokio::test]
async fn bootstrap_creates_owner_once_and_verifies_on_restart() {
    let f = fixture().await;
    let first = f.store.ensure_owner(&f.paths()).await.unwrap();
    assert!(first.created);
    assert_eq!(first.actor.kind, ActorKind::Human);
    let mode = std::fs::metadata(f.paths().owner_token())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let second = f.store.ensure_owner(&f.paths()).await.unwrap();
    assert!(!second.created);
    assert_eq!(second.actor.id, first.actor.id);
    assert_eq!(f.count("credentials").await, 1);
    assert_eq!(f.audit_actions().await, vec!["bootstrapOwner".to_string()]);
}

#[tokio::test]
async fn ensure_owner_never_duplicates_a_revoked_owner_actor() {
    let f = fixture().await;
    let owner = f.owner().await;
    // Out-of-band state (partial restore, manual edit): the human actor
    // lives on but every credential is revoked.
    sqlx::query("UPDATE credentials SET revoked_at = ?1 WHERE actor_id = ?2")
        .bind(format_ts(&f.clock.now()))
        .bind(owner.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

    assert_locked_out(f.store.ensure_owner(&f.paths()).await.unwrap_err());
    assert_eq!(
        f.count("actors").await,
        1,
        "a second owner actor must never be bootstrapped"
    );

    // Reissue recovers by reusing the existing actor.
    let reissued = f.store.reissue_owner_token(&f.paths()).await.unwrap();
    assert_eq!(reissued.actor.id, owner.id);
    assert_eq!(f.count("actors").await, 1);
    f.store.ensure_owner(&f.paths()).await.unwrap();
    let token = f.owner_token().await;
    assert_eq!(
        f.store.authenticate_bearer(&token).await.unwrap().id,
        owner.id
    );
}

#[tokio::test]
async fn missing_owner_token_file_fails_loud_and_reissue_recovers() {
    let f = fixture().await;
    let owner = f.owner().await;
    let old_token = f.owner_token().await;
    std::fs::remove_file(f.paths().owner_token()).unwrap();
    assert_locked_out(f.store.ensure_owner(&f.paths()).await.unwrap_err());
    let reissued = f.store.reissue_owner_token(&f.paths()).await.unwrap();
    assert_eq!(reissued.actor.id, owner.id);
    f.store.ensure_owner(&f.paths()).await.unwrap();
    assert!(matches!(
        f.store.authenticate_bearer(&old_token).await.unwrap_err(),
        DomainError::Unauthenticated
    ));
    let new_token = f.owner_token().await;
    assert_eq!(
        f.store.authenticate_bearer(&new_token).await.unwrap().id,
        owner.id
    );
    assert_eq!(
        f.audit_actions().await,
        vec![
            "bootstrapOwner".to_string(),
            "reissueOwnerToken".to_string()
        ]
    );
}

#[tokio::test]
async fn reissue_invalidates_existing_owner_browser_sessions() {
    let f = fixture().await;
    f.owner().await;
    let stolen = f.owner_token().await;
    let login = f.store.login_browser(&stolen, f.clock.now()).await.unwrap();
    f.store.reissue_owner_token(&f.paths()).await.unwrap();
    // The compromised token's session must not survive the rotation.
    assert!(matches!(
        f.store
            .browser_session(&login.session_token, f.clock.now())
            .await
            .unwrap_err(),
        DomainError::Unauthenticated
    ));
    assert_eq!(f.count("browser_sessions").await, 0);
    let fresh = f.owner_token().await;
    f.store.login_browser(&fresh, f.clock.now()).await.unwrap();
}

#[tokio::test]
async fn failed_reissue_restores_the_previous_token_file() {
    let f = fixture().await;
    f.owner().await;
    let old_token = f.owner_token().await;
    let old_bytes = std::fs::read(f.paths().owner_token()).unwrap();
    // Inject a DB failure inside the rotation: the credential insert is
    // the first statement that writes the new digest.
    sqlx::query(
        "CREATE TRIGGER abort_credential_insert BEFORE INSERT ON credentials \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END",
    )
    .execute(f.store.pool())
    .await
    .unwrap();
    assert!(f.store.reissue_owner_token(&f.paths()).await.is_err());
    assert_eq!(std::fs::read(f.paths().owner_token()).unwrap(), old_bytes);
    // The old credential stays live and the file still verifies on boot.
    assert_eq!(
        f.store.authenticate_bearer(&old_token).await.unwrap().id,
        f.store.ensure_owner(&f.paths()).await.unwrap().actor.id
    );
}

#[tokio::test]
async fn mismatched_owner_token_file_fails_loud() {
    let f = fixture().await;
    f.owner().await;
    write_secret_file(&f.paths().owner_token(), b"not-the-token").unwrap();
    assert_locked_out(f.store.ensure_owner(&f.paths()).await.unwrap_err());
}

#[tokio::test]
async fn create_agent_is_owner_only_and_returns_token_once() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "  builder  ").await;
    assert_eq!(grant.actor.kind, ActorKind::Agent);
    assert_eq!(grant.actor.label, "builder");
    assert_eq!(
        f.store.authenticate_bearer(&grant.token).await.unwrap().id,
        grant.actor.id
    );
    let err = f
        .store
        .create_agent(
            f.ctx(&grant.actor),
            AgentCreate {
                label: "helper".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = f
        .store
        .create_agent(
            f.ctx(&owner),
            AgentCreate {
                label: "  ".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "label", .. }
    ));
    assert_eq!(f.count("actors").await, 2);
    assert_eq!(f.audit_actions().await.last().unwrap(), "createAgent");
}

#[tokio::test]
async fn forged_owner_role_in_context_never_grants_rights() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "builder").await;
    // The caller claims Human in the request body; the DB row is an agent.
    let forged = Actor {
        kind: ActorKind::Human,
        label: "owner".to_string(),
        ..grant.actor.clone()
    };
    let err = f
        .store
        .create_agent(
            f.ctx(&forged),
            AgentCreate {
                label: "minion".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(f.count("actors").await, 2);
}

#[tokio::test]
async fn revoke_agent_atomically_closes_everything_and_replays() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "builder").await;
    // Defensive: a session row for the agent must not survive revocation.
    sqlx::query(
        "INSERT INTO browser_sessions (id, actor_id, token_hash, csrf_hash, \
             csrf_ciphertext, csrf_nonce, created_at, expires_at) \
             VALUES ('sess', ?1, 'hash', 'csrf', x'00', x'00', ?2, '2999-01-01T00:00:00.000Z')",
    )
    .bind(grant.actor.id.to_string())
    .bind(format_ts(&f.clock.now()))
    .execute(f.store.pool())
    .await
    .unwrap();
    let ctx = f.ctx(&owner);
    let key = ctx.idempotency_key;
    let command_id = ctx.command_id;
    let replayed = f
        .store
        .revoke_agent(ctx, grant.actor.id, "rogue".to_string(), "hash-1")
        .await
        .unwrap();
    assert!(!replayed.is_replay());
    assert!(replayed.into_inner().ok);
    assert!(matches!(
        f.store.authenticate_bearer(&grant.token).await.unwrap_err(),
        DomainError::Unauthenticated
    ));
    assert_eq!(f.count("browser_sessions").await, 0);
    let open_credentials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM credentials WHERE actor_id = ?1 AND revoked_at IS NULL",
    )
    .bind(grant.actor.id.to_string())
    .fetch_one(f.store.pool())
    .await
    .unwrap();
    assert_eq!(open_credentials, 0);
    let audits_before = f.count("security_audit").await;

    // Same key and request replays the sealed response without re-running.
    let replay_ctx = CommandContext {
        actor: owner.clone(),
        command_id,
        idempotency_key: key,
        expected_revision: None,
        now: f.clock.now(),
    };
    let replayed = f
        .store
        .revoke_agent(replay_ctx, grant.actor.id, "rogue".to_string(), "hash-1")
        .await
        .unwrap();
    assert!(replayed.is_replay());
    assert_eq!(f.count("security_audit").await, audits_before);

    // Same key, different request: conflict.
    let conflict_ctx = CommandContext {
        actor: owner.clone(),
        command_id,
        idempotency_key: key,
        expected_revision: None,
        now: f.clock.now(),
    };
    let err = f
        .store
        .revoke_agent(conflict_ctx, grant.actor.id, "other".to_string(), "hash-2")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::IdempotencyConflict));

    // Fresh key on an already-revoked agent is terminal.
    let err = f
        .store
        .revoke_agent(f.ctx(&owner), grant.actor.id, "again".to_string(), "hash-3")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));

    // Unknown targets and human targets are not found.
    let err = f
        .store
        .revoke_agent(
            f.ctx(&owner),
            ActorId::generate(f.clock.now()),
            "ghost".to_string(),
            "hash-4",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
    let err = f
        .store
        .revoke_agent(f.ctx(&owner), owner.id, "self".to_string(), "hash-5")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn revoked_actor_fails_before_replay_lookup() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "builder").await;
    let ctx = f.ctx(&grant.actor);
    let key = ctx.idempotency_key;
    let stored = f
        .store
        .idempotent_transaction(&ctx, "hash-1", 200, |_tx| {
            Box::pin(async { Ok(Ack { ok: true }) })
        })
        .await
        .unwrap();
    assert!(!stored.is_replay());
    f.store
        .revoke_agent(f.ctx(&owner), grant.actor.id, "rogue".to_string(), "r")
        .await
        .unwrap();
    let retry = CommandContext {
        actor: grant.actor.clone(),
        command_id: CommandId::generate(f.clock.now()),
        idempotency_key: key,
        expected_revision: None,
        now: f.clock.now(),
    };
    let err = f
        .store
        .idempotent_transaction::<Ack, _>(&retry, "hash-1", 200, |_tx| {
            Box::pin(async { Ok(Ack { ok: true }) })
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    // The revoked actor's stored row survives untouched (owner's revoke adds its own).
    let agent_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM idempotency WHERE actor_id = ?1")
            .bind(grant.actor.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(agent_rows, 1);
}

#[tokio::test]
async fn same_key_different_actor_does_not_replay_foreign_response() {
    let f = fixture().await;
    let owner = f.owner().await;
    let first = f.agent(&owner, "first").await.actor;
    let second = f.agent(&owner, "second").await.actor;
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let ctx = CommandContext {
        actor: first.clone(),
        command_id: CommandId::generate(f.clock.now()),
        idempotency_key: key,
        expected_revision: None,
        now: f.clock.now(),
    };
    let value = f
        .store
        .idempotent_transaction::<i64, _>(&ctx, "hash-1", 200, |_tx| Box::pin(async { Ok(1) }))
        .await
        .unwrap();
    assert!(matches!(value, Replay::Fresh(1)));
    let ctx = CommandContext {
        actor: second.clone(),
        command_id: CommandId::generate(f.clock.now()),
        idempotency_key: key,
        expected_revision: None,
        now: f.clock.now(),
    };
    let value = f
        .store
        .idempotent_transaction::<i64, _>(&ctx, "hash-1", 200, |_tx| Box::pin(async { Ok(2) }))
        .await
        .unwrap();
    // The second actor executes fresh; it never sees the first response.
    assert!(matches!(value, Replay::Fresh(2)));
    assert_eq!(f.count("idempotency").await, 2);
}

#[tokio::test]
async fn browser_sessions_are_fixed_ttl_without_sliding() {
    let f = fixture().await;
    f.owner().await;
    let token = f.owner_token().await;
    let login = f.store.login_browser(&token, f.clock.now()).await.unwrap();
    let expected_expiry = login.session.expires_at;
    f.clock.advance(chrono::TimeDelta::hours(11));
    let session = f
        .store
        .browser_session(&login.session_token, f.clock.now())
        .await
        .unwrap();
    assert_eq!(session.expires_at, expected_expiry);
    assert_eq!(
        session.csrf_token.expose(),
        login.session.csrf_token.expose()
    );
    f.clock.advance(chrono::TimeDelta::hours(2));
    let err = f
        .store
        .browser_session(&login.session_token, f.clock.now())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unauthenticated));
}

#[tokio::test]
async fn login_rejects_agents_and_unknown_tokens_and_prunes_expired_rows() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "builder").await;
    let err = f
        .store
        .login_browser(&grant.token, f.clock.now())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = f
        .store
        .login_browser(&generate_token(), f.clock.now())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unauthenticated));

    let token = f.owner_token().await;
    f.store.login_browser(&token, f.clock.now()).await.unwrap();
    f.clock.advance(chrono::TimeDelta::hours(13));
    f.store.login_browser(&token, f.clock.now()).await.unwrap();
    assert_eq!(f.count("browser_sessions").await, 1);
}

#[tokio::test]
async fn logout_deletes_the_session_row() {
    let f = fixture().await;
    f.owner().await;
    let token = f.owner_token().await;
    let login = f.store.login_browser(&token, f.clock.now()).await.unwrap();
    let ack = f
        .store
        .logout_browser(&login.session_token, f.clock.now())
        .await
        .unwrap();
    assert!(ack.ok);
    assert!(matches!(
        f.store
            .browser_session(&login.session_token, f.clock.now())
            .await
            .unwrap_err(),
        DomainError::Unauthenticated
    ));
    assert!(matches!(
        f.store
            .logout_browser(&login.session_token, f.clock.now())
            .await
            .unwrap_err(),
        DomainError::NotFound
    ));
}

#[tokio::test]
async fn list_agents_paginates_and_rejects_tampered_cursors() {
    let f = fixture().await;
    let owner = f.owner().await;
    let mut ids = Vec::new();
    for label in ["a", "b", "c"] {
        f.clock.advance(chrono::TimeDelta::seconds(1));
        ids.push(f.agent(&owner, label).await.actor.id);
    }
    f.store
        .revoke_agent(f.ctx(&owner), ids[0], "rogue".to_string(), "hash")
        .await
        .unwrap();
    let page = f
        .store
        .list_agents(
            &owner.id,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    // Revoked agents remain listed for owner review.
    assert!(page.items[0].revoked);
    let cursor = page.next_cursor.expect("second page exists");
    let rest = f
        .store
        .list_agents(
            &owner.id,
            &ListParams {
                limit: Some(2),
                cursor: Some(cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert_eq!(rest.items[0].id, ids[2]);
    assert!(rest.next_cursor.is_none());

    let err = f
        .store
        .list_agents(
            &owner.id,
            &ListParams {
                cursor: Some(format!("{cursor}x")),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));
    let err = f
        .store
        .list_agents(
            &owner.id,
            &ListParams {
                limit: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "limit", .. }
    ));
}

#[tokio::test]
async fn list_agents_is_owner_only_in_core() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f.agent(&owner, "builder").await;
    let params = ListParams::default();
    assert!(matches!(
        f.store.list_agents(&grant.actor.id, &params).await,
        Err(DomainError::Forbidden(_))
    ));
    // A revoked agent gets the same refusal, not a session of its data.
    f.store
        .revoke_agent(f.ctx(&owner), grant.actor.id, "rogue".to_string(), "hash")
        .await
        .unwrap();
    assert!(matches!(
        f.store.list_agents(&grant.actor.id, &params).await,
        Err(DomainError::Forbidden(_))
    ));
    // An unknown caller is rejected too, before any agent rows are read.
    let unknown = ActorId::generate(f.clock.now());
    assert!(matches!(
        f.store.list_agents(&unknown, &params).await,
        Err(DomainError::Forbidden(_))
    ));
    let page = f.store.list_agents(&owner.id, &params).await.unwrap();
    assert_eq!(page.items.len(), 1);
}
