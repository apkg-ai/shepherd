use sqlx::{QueryBuilder, Sqlite, SqliteConnection};

use std::collections::{BTreeMap, BTreeSet};

use super::credentials::{actor_from_joined_row, insert_credential, security_audit};
use crate::commands::{
    ClaimScope, CommandContext, CommandResult, PendingEvent, Replay, append_events, live_actor,
    missing_after_write, revoke_active_claims,
};
use crate::error::DomainError;
use crate::model::{
    Ack, Actor, ActorId, ActorKind, AgentCreate, AgentTokenGrant, Capability, NAME_MAX_CHARS,
    REASON_MAX_CHARS, generate_token, require_capability, token_digest, validate_required_text,
};
use crate::model::{EpicId, ProjectId, TaskId};
use crate::queries::{ListParams, Page, decode_cursor, effective_limit, encode_cursor, split_page};
use crate::storage::rows::{format_ts, insert_actor, parse_uuid};
use crate::storage::{StorageError, Store};

const AGENTS_ENDPOINT: &str = "listAgents";
const AGENTS_FILTER: &str = "kind=agent";

/// task -> (project_id text, epic_id), batched for cross-project event grouping.
async fn task_event_scopes(
    conn: &mut SqliteConnection,
    tasks: &BTreeSet<TaskId>,
) -> Result<BTreeMap<TaskId, (String, EpicId)>, DomainError> {
    let mut scopes = BTreeMap::new();
    let ids: Vec<&TaskId> = tasks.iter().collect();
    for chunk in ids.chunks(500) {
        let mut builder =
            QueryBuilder::<Sqlite>::new("SELECT id, project_id, epic_id FROM tasks WHERE id IN (");
        let mut separated = builder.separated(", ");
        for task in chunk {
            separated.push_bind(task.to_string());
        }
        builder.push(")");
        let rows: Vec<(String, String, String)> = builder
            .build_query_as()
            .fetch_all(&mut *conn)
            .await
            .map_err(StorageError::from)?;
        for (id, project, epic) in rows {
            scopes.insert(
                TaskId::from_uuid(parse_uuid("tasks.id", &id)?),
                (
                    project,
                    EpicId::from_uuid(parse_uuid("tasks.epic_id", &epic)?),
                ),
            );
        }
    }
    Ok(scopes)
}

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

    /// Atomically marks the actor revoked and closes its credentials, browser
    /// sessions and active claims (plan/03: reviews and reports are preserved).
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
                let closed =
                    revoke_active_claims(tx, ClaimScope::Actor(agent), &now, &reason).await?;
                // Claims may span projects; events group per project. Revoked
                // claims change their task's representation, so those tasks
                // bump and emit task.changed (same as workflow::epic::block).
                let tasks: BTreeSet<TaskId> = closed.all().map(|claim| claim.task_id).collect();
                let scopes = task_event_scopes(tx, &tasks).await?;
                let scope_of =
                    |task: &TaskId| scopes.get(task).ok_or_else(|| missing_after_write("task"));
                let mut by_project: BTreeMap<String, Vec<PendingEvent>> = BTreeMap::new();
                for claim in closed.all() {
                    let (project, _) = scope_of(&claim.task_id)?;
                    by_project
                        .entry(project.clone())
                        .or_default()
                        .push(PendingEvent::claim(
                            claim.id,
                            claim.task_revision,
                            claim.task_id,
                        ));
                }
                let revoked: Vec<TaskId> =
                    closed.revoked.iter().map(|claim| claim.task_id).collect();
                for (task, revision) in
                    crate::workflow::epic::bump_tasks(tx, &revoked, &now).await?
                {
                    let (project, epic) = scope_of(&task)?;
                    by_project
                        .entry(project.clone())
                        .or_default()
                        .push(PendingEvent::task(task, revision, *epic));
                }
                let events_ctx = CommandContext::for_events(actor, command_id, at);
                for (project, events) in by_project {
                    let project_id =
                        ProjectId::from_uuid(parse_uuid("tasks.project_id", &project)?);
                    append_events(tx, &project_id, &events_ctx, "revokeAgent", &reason, events)
                        .await?;
                }
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
            // The held connection: acquiring a second one here could hold every
            // pool connection at the authorization stage and stall concurrent
            // page queries until the acquire timeout.
            .fetch_all(&mut *conn)
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

    /// Plain actor read including the revoked flag; long-lived consumers such
    /// as the SSE stream recheck revocation with it (plan/08).
    pub async fn get_actor(&self, id: &ActorId) -> Result<Option<Actor>, DomainError> {
        Ok(crate::storage::rows::get_actor(self.pool(), id).await?)
    }
}
