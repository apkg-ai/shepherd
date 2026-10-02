use sqlx::{QueryBuilder, Sqlite};

use super::credentials::{actor_from_joined_row, insert_credential, security_audit};
use crate::commands::{CommandContext, CommandResult, Replay, live_actor};
use crate::error::DomainError;
use crate::model::{
    Ack, Actor, ActorId, ActorKind, AgentCreate, AgentTokenGrant, Capability, NAME_MAX_CHARS,
    REASON_MAX_CHARS, generate_token, require_capability, token_digest, validate_required_text,
};
use crate::queries::{ListParams, Page, decode_cursor, effective_limit, encode_cursor, split_page};
use crate::storage::rows::{format_ts, insert_actor};
use crate::storage::{StorageError, Store};

const AGENTS_ENDPOINT: &str = "listAgents";
const AGENTS_FILTER: &str = "kind=agent";

impl Store {
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

    /// Agents including revoked ones, keyset-paginated like every list (plan/07).
    /// Owner-only in core, not just in the HTTP gate: future CLI/MCP callers
    /// must not bypass the authorization boundary.
    pub async fn list_agents(
        &self,
        caller: &ActorId,
        params: &ListParams,
    ) -> Result<Page<Actor>, DomainError> {
        let mut conn = self.pool().acquire().await.map_err(StorageError::from)?;
        let actor = live_actor(&mut conn, caller).await?;
        require_capability(&actor, Capability::ManageCredentials)?;
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
