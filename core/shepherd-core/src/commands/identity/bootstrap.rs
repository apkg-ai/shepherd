use std::path::{Path, PathBuf};

use super::credentials::{insert_credential, live_owner_credential_or_actor, security_audit};
use crate::error::DomainError;
use crate::model::{
    Actor, ActorId, ActorKind, CommandId, OwnerBootstrap, SecretString, digest_matches,
    generate_token, token_digest,
};
use crate::storage::rows::{format_ts, insert_actor};
use crate::storage::{StorageError, Store, verify_secret_file_mode, write_secret_file};

const OWNER_LABEL: &str = "owner";

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
        let existing = live_owner_credential_or_actor(self.pool()).await?;
        let token_path = paths.owner_token();
        let Some((actor, credential)) = existing else {
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
            let registered = owner.clone();
            self.domain_transaction(move |tx| {
                Box::pin(async move {
                    insert_actor(tx, &registered).await?;
                    insert_credential(tx, &registered.id, &digest, &now).await?;
                    security_audit(
                        tx,
                        &registered.id,
                        "bootstrapOwner",
                        None,
                        "",
                        &command_id,
                        &now,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await?;
            return Ok(OwnerBootstrap {
                actor: owner,
                created: true,
            });
        };
        let Some(stored_hash) = credential else {
            // An owner actor exists but every credential is revoked: never
            // bootstrap a second owner principal over it; reissue recovers.
            return Err(locked_out(
                &token_path,
                "cannot be verified while every owner credential is revoked",
            ));
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
    /// token file. The owner Actor never changes identity (plan/12). A failed
    /// commit restores the previous file so the old credential stays usable.
    pub async fn reissue_owner_token(
        &self,
        paths: &IdentityPaths,
    ) -> Result<OwnerBootstrap, DomainError> {
        let Some((actor, _)) = live_owner_credential_or_actor(self.pool()).await? else {
            return self.ensure_owner(paths).await;
        };
        let token = generate_token();
        let token_path = paths.owner_token();
        let previous_file = std::fs::read(&token_path).ok();
        write_secret_file(&token_path, token.expose().as_bytes())?;
        let digest = token_digest(&token);
        let now = self.clock().now();
        let command_id = CommandId::generate(now);
        let owner_id = actor.id;
        let rotated = self
            .domain_transaction(move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE credentials SET revoked_at = ?1 WHERE revoked_at IS NULL \
                     AND actor_id IN (SELECT id FROM actors WHERE kind = 'human')",
                    )
                    .bind(format_ts(&now))
                    .execute(&mut **tx)
                    .await?;
                    // Reissue is the compromise-recovery path: sessions minted with
                    // the old token must die with it, like revocation does for agents.
                    sqlx::query(
                        "DELETE FROM browser_sessions \
                     WHERE actor_id IN (SELECT id FROM actors WHERE kind = 'human')",
                    )
                    .execute(&mut **tx)
                    .await?;
                    insert_credential(tx, &owner_id, &digest, &now).await?;
                    security_audit(
                        tx,
                        &owner_id,
                        "reissueOwnerToken",
                        None,
                        "",
                        &command_id,
                        &now,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await;
        if rotated.is_err() {
            // The rollback kept the old credential live; put its token back so
            // the file still matches the DB and the next startup verifies.
            match &previous_file {
                Some(previous) => {
                    let _ = write_secret_file(&token_path, previous);
                }
                None => {
                    let _ = std::fs::remove_file(&token_path);
                }
            }
        }
        rotated?;
        Ok(OwnerBootstrap {
            actor,
            created: false,
        })
    }
}
