use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, Transaction};

use super::{CommandContext, CommandResult, Replay, live_actor};
use crate::error::DomainError;
use crate::model::{
    Ack, Actor, ActorId, ActorKind, AgentCreate, AgentTokenGrant, BrowserSession,
    BrowserSessionGrant, Capability, CommandId, NAME_MAX_CHARS, OwnerBootstrap, REASON_MAX_CHARS,
    SESSION_TTL_SECONDS, SecretString, digest_matches, generate_token, require_capability,
    token_digest, validate_required_text,
};
use crate::queries::{ListParams, Page, decode_cursor, effective_limit, encode_cursor, split_page};
use crate::storage::rows::{format_ts, insert_actor, parse_flag, parse_ts, parse_uuid};
use crate::storage::{
    SealedResponse, StorageError, Store, verify_secret_file_mode, write_secret_file,
};

const OWNER_LABEL: &str = "owner";
const AGENTS_ENDPOINT: &str = "listAgents";
const AGENTS_FILTER: &str = "kind=agent";

/// Identity file locations inside the data directory (plan/13).
pub struct IdentityPaths {
    data_dir: PathBuf,
}

impl IdentityPaths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn owner_token(&self) -> PathBuf {
        self.data_dir.join("owner-token")
    }

    pub fn replay_key(&self) -> PathBuf {
        self.data_dir.join("replay-key")
    }

    pub fn daemon_lock(&self) -> PathBuf {
        self.data_dir.join("daemon.lock")
    }
}

fn locked_out(path: &Path, problem: &str) -> DomainError {
    StorageError::CredentialFile(format!(
        "owner token file {} {problem}; restart with --reissue-owner-token to rotate the owner credential",
        path.display()
    ))
    .into()
}

impl Store {
    /// First startup creates the owner actor and token file; later startups
    /// verify the file against the stored digest. The file is written before
    /// the credential row so a crash between the two self-heals on next boot.
    pub async fn ensure_owner(&self, paths: &IdentityPaths) -> Result<OwnerBootstrap, DomainError> {
        let existing = live_owner_credential(self.pool()).await?;
        let token_path = paths.owner_token();
        let Some((actor, stored_hash)) = existing else {
            let token = generate_token();
            write_secret_file(&token_path, token.expose().as_bytes())?;
            let now = self.clock().now();
            let owner = Actor {
                id: ActorId::generate(now),
                kind: ActorKind::Human,
                label: OWNER_LABEL.to_string(),
                revoked: false,
                created_at: now,
            };
            let digest = token_digest(&token);
            let command_id = CommandId::generate(now);
            let mut tx = self.begin_command().await?;
            let result: Result<(), DomainError> = async {
                insert_actor(&mut tx, &owner).await?;
                insert_credential(&mut tx, &owner.id, &digest, &now).await?;
                security_audit(
                    &mut tx,
                    &owner.id,
                    "bootstrapOwner",
                    None,
                    "",
                    &command_id,
                    &now,
                )
                .await?;
                Ok(())
            }
            .await;
            commit_or_rollback(tx, result).await?;
            return Ok(OwnerBootstrap {
                actor: owner,
                created: true,
            });
        };
        if !token_path.exists() {
            return Err(locked_out(
                &token_path,
                "is missing while an owner credential exists",
            ));
        }
        verify_secret_file_mode(&token_path)?;
        let contents = std::fs::read_to_string(&token_path).map_err(StorageError::from)?;
        let file_token = SecretString::new(contents.trim_end().to_string());
        if !digest_matches(&token_digest(&file_token), &stored_hash) {
            return Err(locked_out(
                &token_path,
                "does not match the stored owner credential",
            ));
        }
        Ok(OwnerBootstrap {
            actor,
            created: false,
        })
    }

    /// Explicit recovery path: rotates the owner credential and rewrites the
    /// token file. The owner Actor never changes identity (plan/12).
    pub async fn reissue_owner_token(
        &self,
        paths: &IdentityPaths,
    ) -> Result<OwnerBootstrap, DomainError> {
        let Some((actor, _)) = live_owner_credential_or_actor(self.pool()).await? else {
            return self.ensure_owner(paths).await;
        };
        let token = generate_token();
        write_secret_file(&paths.owner_token(), token.expose().as_bytes())?;
        let digest = token_digest(&token);
        let now = self.clock().now();
        let command_id = CommandId::generate(now);
        let owner_id = actor.id;
        let mut tx = self.begin_command().await?;
        let result: Result<(), DomainError> = async {
            sqlx::query(
                "UPDATE credentials SET revoked_at = ?1 WHERE revoked_at IS NULL \
                 AND actor_id IN (SELECT id FROM actors WHERE kind = 'human')",
            )
            .bind(format_ts(&now))
            .execute(&mut *tx)
            .await?;
            insert_credential(&mut tx, &owner_id, &digest, &now).await?;
            security_audit(
                &mut tx,
                &owner_id,
                "reissueOwnerToken",
                None,
                "",
                &command_id,
                &now,
            )
            .await?;
            Ok(())
        }
        .await;
        commit_or_rollback(tx, result).await?;
        Ok(OwnerBootstrap {
            actor,
            created: false,
        })
    }

    /// Owner-only issuance: a distinct agent Actor plus a token returned once.
    pub async fn create_agent(
        &self,
        ctx: CommandContext,
        input: AgentCreate,
    ) -> Result<CommandResult<AgentTokenGrant>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                require_capability(&actor, Capability::ManageCredentials)?;
                let label = validate_required_text("label", &input.label, NAME_MAX_CHARS)?;
                let agent = Actor {
                    id: ActorId::generate(ctx.now),
                    kind: ActorKind::Agent,
                    label,
                    revoked: false,
                    created_at: ctx.now,
                };
                let token = generate_token();
                insert_actor(&mut *tx, &agent).await?;
                insert_credential(&mut *tx, &agent.id, &token_digest(&token), &ctx.now).await?;
                security_audit(
                    &mut *tx,
                    &ctx.actor.id,
                    "createAgent",
                    Some(&agent.id),
                    "",
                    &ctx.command_id,
                    &ctx.now,
                )
                .await?;
                Ok(CommandResult {
                    value: AgentTokenGrant {
                        actor: agent,
                        token,
                    },
                    events: vec![],
                })
            })
        })
        .await
    }

    /// Atomically marks the actor revoked and closes its credentials and
    /// browser sessions. Claim closure joins in step 009 when claims exist.
    pub async fn revoke_agent(
        &self,
        ctx: CommandContext,
        agent: ActorId,
        reason: String,
        request_hash: &str,
    ) -> Result<Replay<Ack>, DomainError> {
        let caller = ctx.actor.id;
        let command_id = ctx.command_id;
        let at = ctx.now;
        self.idempotent_transaction(&ctx, request_hash, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &caller).await?;
                require_capability(&actor, Capability::ManageCredentials)?;
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let target = crate::storage::rows::get_actor(&mut **tx, &agent)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if target.kind != ActorKind::Agent {
                    // The owner is never revocable through this route.
                    return Err(DomainError::NotFound);
                }
                if target.revoked {
                    return Err(DomainError::TerminalScope);
                }
                let now = format_ts(&at);
                sqlx::query("UPDATE actors SET revoked = 1 WHERE id = ?1")
                    .bind(agent.to_string())
                    .execute(&mut **tx)
                    .await?;
                sqlx::query(
                    "UPDATE credentials SET revoked_at = ?2 WHERE actor_id = ?1 \
                     AND revoked_at IS NULL",
                )
                .bind(agent.to_string())
                .bind(&now)
                .execute(&mut **tx)
                .await?;
                // Defensive: agents cannot hold browser sessions, but revocation
                // must leave none behind.
                sqlx::query("DELETE FROM browser_sessions WHERE actor_id = ?1")
                    .bind(agent.to_string())
                    .execute(&mut **tx)
                    .await?;
                security_audit(
                    &mut *tx,
                    &caller,
                    "revokeAgent",
                    Some(&agent),
                    &reason,
                    &command_id,
                    &at,
                )
                .await?;
                Ok(Ack { ok: true })
            })
        })
        .await
    }

    /// Browser login: owner token in, session cookie + CSRF out (plan/12).
    /// Not replayable and not audited (plan/07).
    pub async fn login_browser(
        &self,
        owner_token: &SecretString,
        now: DateTime<Utc>,
    ) -> Result<BrowserSessionGrant, DomainError> {
        let digest = token_digest(owner_token);
        let mut tx = self.begin_command().await?;
        let result = self.login_browser_body(&mut tx, &digest, now).await;
        commit_or_rollback(tx, result).await
    }

    async fn login_browser_body(
        &self,
        tx: &mut Transaction<'static, Sqlite>,
        digest: &str,
        now: DateTime<Utc>,
    ) -> Result<BrowserSessionGrant, DomainError> {
        let Some((actor, stored_hash)) = credential_actor_by_hash(&mut **tx, digest).await? else {
            return Err(DomainError::Unauthenticated);
        };
        if !digest_matches(digest, &stored_hash) {
            return Err(DomainError::Unauthenticated);
        }
        if actor.revoked {
            return Err(DomainError::Unauthenticated);
        }
        if actor.kind != ActorKind::Human {
            return Err(DomainError::Forbidden(
                "agents cannot create browser sessions".into(),
            ));
        }
        sqlx::query("DELETE FROM browser_sessions WHERE actor_id = ?1 AND expires_at <= ?2")
            .bind(actor.id.to_string())
            .bind(format_ts(&now))
            .execute(&mut **tx)
            .await?;
        let session_id = crate::model::new_v7(now).to_string();
        let session_token = generate_token();
        let csrf_token = generate_token();
        let sealed = self
            .codec()
            .seal_bytes(session_id.as_bytes(), csrf_token.expose().as_bytes())?;
        let expires_at = now + TimeDelta::seconds(SESSION_TTL_SECONDS);
        sqlx::query(
            "INSERT INTO browser_sessions (id, actor_id, token_hash, csrf_hash, \
             csrf_ciphertext, csrf_nonce, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&session_id)
        .bind(actor.id.to_string())
        .bind(token_digest(&session_token))
        .bind(token_digest(&csrf_token))
        .bind(&sealed.ciphertext)
        .bind(&sealed.nonce)
        .bind(format_ts(&now))
        .bind(format_ts(&expires_at))
        .execute(&mut **tx)
        .await?;
        Ok(BrowserSessionGrant {
            session_token,
            session: BrowserSession {
                actor,
                csrf_token,
                expires_at,
            },
        })
    }

    /// Cookie authentication. Returns the stored CSRF and the fixed expiry;
    /// no sliding extension and no write side effects (plan/05, plan/12).
    pub async fn browser_session(
        &self,
        session_token: &SecretString,
        now: DateTime<Utc>,
    ) -> Result<BrowserSession, DomainError> {
        let row = sqlx::query(
            "SELECT s.id AS session_id, s.csrf_ciphertext, s.csrf_nonce, s.expires_at, \
             a.id, a.kind, a.label, a.revoked, a.created_at \
             FROM browser_sessions s JOIN actors a ON a.id = s.actor_id \
             WHERE s.token_hash = ?1 AND s.expires_at > ?2",
        )
        .bind(token_digest(session_token))
        .bind(format_ts(&now))
        .fetch_optional(self.pool())
        .await
        .map_err(StorageError::from)?
        .ok_or(DomainError::Unauthenticated)?;
        let actor = actor_from_joined_row(&row)?;
        if actor.revoked {
            return Err(DomainError::Unauthenticated);
        }
        let session_id: String = row.try_get("session_id").map_err(StorageError::from)?;
        let expires_at: String = row.try_get("expires_at").map_err(StorageError::from)?;
        let sealed = SealedResponse {
            ciphertext: row.try_get("csrf_ciphertext").map_err(StorageError::from)?,
            nonce: row.try_get("csrf_nonce").map_err(StorageError::from)?,
        };
        let csrf = self.codec().open_bytes(session_id.as_bytes(), &sealed)?;
        let csrf = String::from_utf8(csrf)
            .map_err(|_| StorageError::Corrupt("browser_sessions.csrf_ciphertext".into()))?;
        Ok(BrowserSession {
            actor,
            csrf_token: SecretString::new(csrf),
            expires_at: parse_ts("browser_sessions.expires_at", &expires_at)?,
        })
    }

    /// Logout deletes the session row; expired rows still delete (plan/12).
    pub async fn logout_browser(
        &self,
        session_token: &SecretString,
        _now: DateTime<Utc>,
    ) -> Result<Ack, DomainError> {
        let digest = token_digest(session_token);
        let mut tx = self.begin_command().await?;
        let result: Result<Ack, DomainError> = async {
            let deleted = sqlx::query("DELETE FROM browser_sessions WHERE token_hash = ?1")
                .bind(&digest)
                .execute(&mut *tx)
                .await?;
            if deleted.rows_affected() == 0 {
                return Err(DomainError::NotFound);
            }
            Ok(Ack { ok: true })
        }
        .await;
        commit_or_rollback(tx, result).await
    }

    /// Bearer authentication: live credential, live actor, nothing from the body.
    pub async fn authenticate_bearer(&self, token: &SecretString) -> Result<Actor, DomainError> {
        let digest = token_digest(token);
        let Some((actor, stored_hash)) = credential_actor_by_hash(self.pool(), &digest).await?
        else {
            return Err(DomainError::Unauthenticated);
        };
        if !digest_matches(&digest, &stored_hash) || actor.revoked {
            return Err(DomainError::Unauthenticated);
        }
        Ok(actor)
    }

    /// Agents including revoked ones, keyset-paginated like every list (plan/07).
    pub async fn list_agents(&self, params: &ListParams) -> Result<Page<Actor>, DomainError> {
        let after = params
            .cursor
            .as_deref()
            .map(|cursor| decode_cursor(cursor, AGENTS_ENDPOINT, AGENTS_FILTER))
            .transpose()?;
        let limit = effective_limit(params)?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, kind, label, revoked, created_at FROM actors WHERE kind = 'agent'",
        );
        if let Some((created_at, id)) = &after {
            builder
                .push(" AND (created_at > ")
                .push_bind(created_at.clone())
                .push(" OR (created_at = ")
                .push_bind(created_at.clone())
                .push(" AND id > ")
                .push_bind(id.clone())
                .push("))");
        }
        builder
            .push(" ORDER BY created_at ASC, id ASC LIMIT ")
            .push_bind(limit + 1);
        let rows = builder
            .build()
            .fetch_all(self.pool())
            .await
            .map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(actor_from_joined_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |actor| {
            encode_cursor(
                AGENTS_ENDPOINT,
                AGENTS_FILTER,
                &format_ts(&actor.created_at),
                &actor.id.to_string(),
            )
        }))
    }
}

async fn commit_or_rollback<T>(
    tx: Transaction<'static, Sqlite>,
    result: Result<T, DomainError>,
) -> Result<T, DomainError> {
    match result {
        Ok(value) => {
            tx.commit().await.map_err(StorageError::from)?;
            Ok(value)
        }
        Err(err) => {
            tx.rollback().await.ok();
            Err(err)
        }
    }
}

async fn insert_credential(
    conn: &mut SqliteConnection,
    actor: &ActorId,
    token_hash: &str,
    now: &DateTime<Utc>,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO credentials (id, actor_id, token_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(crate::model::new_v7(*now).to_string())
    .bind(actor.to_string())
    .bind(token_hash)
    .bind(format_ts(now))
    .execute(conn)
    .await?;
    Ok(())
}

async fn security_audit(
    conn: &mut SqliteConnection,
    actor: &ActorId,
    action: &str,
    target: Option<&ActorId>,
    reason: &str,
    command_id: &CommandId,
    now: &DateTime<Utc>,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO security_audit (actor_id, action, target_actor_id, reason, command_id, \
         created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(actor.to_string())
    .bind(action)
    .bind(target.map(ActorId::to_string))
    .bind(reason)
    .bind(command_id.to_string())
    .bind(format_ts(now))
    .execute(conn)
    .await?;
    Ok(())
}

fn actor_from_joined_row(row: &SqliteRow) -> Result<Actor, DomainError> {
    let id: String = row.try_get("id").map_err(StorageError::from)?;
    let kind: String = row.try_get("kind").map_err(StorageError::from)?;
    let created_at: String = row.try_get("created_at").map_err(StorageError::from)?;
    Ok(Actor {
        id: ActorId::from_uuid(parse_uuid("actors.id", &id)?),
        kind: ActorKind::parse(&kind)
            .ok_or_else(|| StorageError::Corrupt(format!("actors.kind: {kind:?}")))?,
        label: row.try_get("label").map_err(StorageError::from)?,
        revoked: parse_flag(
            "actors.revoked",
            row.try_get("revoked").map_err(StorageError::from)?,
        )?,
        created_at: parse_ts("actors.created_at", &created_at)?,
    })
}

async fn credential_actor_by_hash<'e, E>(
    executor: E,
    digest: &str,
) -> Result<Option<(Actor, String)>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(row) = sqlx::query(
        "SELECT a.id, a.kind, a.label, a.revoked, a.created_at, c.token_hash \
         FROM credentials c JOIN actors a ON a.id = c.actor_id \
         WHERE c.token_hash = ?1 AND c.revoked_at IS NULL",
    )
    .bind(digest)
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    let hash: String = row.try_get("token_hash").map_err(StorageError::from)?;
    Ok(Some((actor_from_joined_row(&row)?, hash)))
}

async fn live_owner_credential<'e, E>(executor: E) -> Result<Option<(Actor, String)>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(row) = sqlx::query(
        "SELECT a.id, a.kind, a.label, a.revoked, a.created_at, c.token_hash \
         FROM credentials c JOIN actors a ON a.id = c.actor_id \
         WHERE a.kind = 'human' AND c.revoked_at IS NULL \
         ORDER BY c.created_at DESC, c.id DESC LIMIT 1",
    )
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    let hash: String = row.try_get("token_hash").map_err(StorageError::from)?;
    Ok(Some((actor_from_joined_row(&row)?, hash)))
}

// Reissue also recovers an owner whose credentials were all revoked.
async fn live_owner_credential_or_actor<'e, E>(
    executor: E,
) -> Result<Option<(Actor, Option<String>)>, DomainError>
where
    E: sqlx::Executor<'e, Database = Sqlite> + Copy,
{
    if let Some((actor, hash)) = live_owner_credential(executor).await? {
        return Ok(Some((actor, Some(hash))));
    }
    let Some(row) = sqlx::query(
        "SELECT id, kind, label, revoked, created_at FROM actors \
         WHERE kind = 'human' ORDER BY created_at ASC, id ASC LIMIT 1",
    )
    .fetch_optional(executor)
    .await
    .map_err(StorageError::from)?
    else {
        return Ok(None);
    };
    Ok(Some((actor_from_joined_row(&row)?, None)))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::model::{Clock, TestClock};
    use crate::storage::open;
    use crate::storage::testing::{store_options, test_clock};

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
            .list_agents(&ListParams {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        // Revoked agents remain listed for owner review.
        assert!(page.items[0].revoked);
        let cursor = page.next_cursor.expect("second page exists");
        let rest = f
            .store
            .list_agents(&ListParams {
                limit: Some(2),
                cursor: Some(cursor.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.items[0].id, ids[2]);
        assert!(rest.next_cursor.is_none());

        let err = f
            .store
            .list_agents(&ListParams {
                cursor: Some(format!("{cursor}x")),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidCursor(_)));
        let err = f
            .store
            .list_agents(&ListParams {
                limit: Some(0),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "limit", .. }
        ));
    }
}
