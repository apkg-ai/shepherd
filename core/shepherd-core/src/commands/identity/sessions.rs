use chrono::{DateTime, TimeDelta, Utc};
use sqlx::Row;

use super::credentials::{actor_from_joined_row, credential_actor_by_hash};
use crate::error::DomainError;
use crate::model::{
    Ack, Actor, ActorKind, BrowserSession, BrowserSessionGrant, SESSION_TTL_SECONDS, SecretString,
    generate_token, token_digest,
};
use crate::storage::rows::{format_ts, parse_ts};
use crate::storage::{SealedResponse, StorageError, Store};

impl Store {
    /// Browser login: owner token in, session cookie + CSRF out (plan/12).
    /// Not replayable and not audited (plan/07).
    pub async fn login_browser(
        &self,
        owner_token: &SecretString,
        now: DateTime<Utc>,
    ) -> Result<BrowserSessionGrant, DomainError> {
        let digest = token_digest(owner_token);
        let codec = self.codec_arc();
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let Some(actor) = credential_actor_by_hash(&mut **tx, &digest).await? else {
                    return Err(DomainError::Unauthenticated);
                };
                if actor.revoked {
                    return Err(DomainError::Unauthenticated);
                }
                if actor.kind != ActorKind::Human {
                    return Err(DomainError::Forbidden(
                        "agents cannot create browser sessions".into(),
                    ));
                }
                sqlx::query(
                    "DELETE FROM browser_sessions WHERE actor_id = ?1 AND expires_at <= ?2",
                )
                .bind(actor.id.to_string())
                .bind(format_ts(&now))
                .execute(&mut **tx)
                .await?;
                let session_id = crate::model::new_v7(now).to_string();
                let session_token = generate_token();
                let csrf_token = generate_token();
                let sealed =
                    codec.seal_bytes(session_id.as_bytes(), csrf_token.expose().as_bytes())?;
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
            })
        })
        .await
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
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let deleted = sqlx::query("DELETE FROM browser_sessions WHERE token_hash = ?1")
                    .bind(&digest)
                    .execute(&mut **tx)
                    .await?;
                if deleted.rows_affected() == 0 {
                    return Err(DomainError::NotFound);
                }
                Ok(Ack { ok: true })
            })
        })
        .await
    }

    /// Bearer authentication: live credential, live actor, nothing from the body.
    pub async fn authenticate_bearer(&self, token: &SecretString) -> Result<Actor, DomainError> {
        let digest = token_digest(token);
        let Some(actor) = credential_actor_by_hash(self.pool(), &digest).await? else {
            return Err(DomainError::Unauthenticated);
        };
        if actor.revoked {
            return Err(DomainError::Unauthenticated);
        }
        Ok(actor)
    }
}
