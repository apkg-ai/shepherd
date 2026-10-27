use shepherd_core::model::{ActorId, AgentCreate as CoreAgentCreate, SecretString};
use uuid::Uuid;

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, require_principal,
    require_session, require_store,
};
use crate::convert::{command_context, list_params, request_hash, request_id, wire_actor};
use crate::generated::server::api::IdentityApi;
use crate::generated::server::errors::{
    CreateAgentResponse, CreateBrowserSessionResponse, DeleteBrowserSessionResponse,
    GetBrowserSessionResponse, GetPrincipalResponse, ListAgentsResponse, RevokeAgentResponse,
};
use crate::generated::types as wire;
use crate::middleware::{CookieAction, current_context, problem, set_cookie_action};
use axum::http::StatusCode;

#[async_trait::async_trait]
impl IdentityApi for AppState {
    async fn create_browser_session(
        &self,
        body: wire::SessionLogin,
    ) -> CreateBrowserSessionResponse {
        let store = require_store!(self, CreateBrowserSessionResponse);
        let token = SecretString::new(body.owner_token);
        let now = store.clock().now();
        match store.login_browser(&token, now).await {
            Ok(grant) => {
                set_cookie_action(CookieAction::Set(grant.session_token));
                CreateBrowserSessionResponse::Created(wire::BrowserSession {
                    actor: wire_actor(&grant.session.actor),
                    csrf_token: grant.session.csrf_token.expose().to_string(),
                    expires_at: grant.session.expires_at,
                })
            }
            Err(err) => domain_problem!(CreateBrowserSessionResponse, err),
        }
    }

    async fn get_browser_session(&self) -> GetBrowserSessionResponse {
        let ctx = current_context();
        // The gate already authenticated the cookie; re-serve the stored CSRF
        // and the fixed expiry (no sliding extension, plan/12).
        match &ctx.0.session {
            Some((_, session)) => GetBrowserSessionResponse::Ok(wire::BrowserSession {
                actor: wire_actor(&session.actor),
                csrf_token: session.csrf_token.expose().to_string(),
                expires_at: session.expires_at,
            }),
            None => GetBrowserSessionResponse::NotFound(problem(
                StatusCode::NOT_FOUND,
                "not_found",
                "no browser session for this credential",
                request_id(),
            )),
        }
    }

    async fn delete_browser_session(
        &self,
        _x_csrf_token: Option<String>,
    ) -> DeleteBrowserSessionResponse {
        let store = require_store!(self, DeleteBrowserSessionResponse);
        let (session_token, _) = require_session!(DeleteBrowserSessionResponse);
        let now = store.clock().now();
        match store.logout_browser(&session_token, now).await {
            Ok(ack) => {
                set_cookie_action(CookieAction::Clear);
                DeleteBrowserSessionResponse::Ok(wire::Ack { ok: ack.ok })
            }
            Err(err) => domain_problem!(DeleteBrowserSessionResponse, err),
        }
    }

    async fn list_agents(&self, cursor: Option<String>, limit: Option<i32>) -> ListAgentsResponse {
        let store = require_store!(self, ListAgentsResponse);
        let principal = require_principal!(ListAgentsResponse);
        match store
            .list_agents(&principal.id, &list_params(cursor, limit, None))
            .await
        {
            Ok(page) => ListAgentsResponse::Ok(wire::PageActor {
                items: page.items.iter().map(wire_actor).collect(),
                next_cursor: page.next_cursor,
            }),
            Err(err) => domain_problem!(ListAgentsResponse, err),
        }
    }

    async fn create_agent(
        &self,
        _x_csrf_token: Option<String>,
        body: wire::AgentCreate,
    ) -> CreateAgentResponse {
        let store = require_store!(self, CreateAgentResponse);
        let principal = require_principal!(CreateAgentResponse);
        // Issuance is not replayable (plan/12): fresh key each call.
        let ctx = command_context(
            &principal,
            Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            None,
            None,
            store,
        );
        match store
            .create_agent(ctx, CoreAgentCreate { label: body.label })
            .await
        {
            Ok(result) => CreateAgentResponse::Created(wire::AgentToken {
                actor: wire_actor(&result.value.actor),
                token: result.value.token.expose().to_string(),
            }),
            Err(err) => domain_problem!(CreateAgentResponse, err),
        }
    }

    async fn revoke_agent(
        &self,
        agent_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> RevokeAgentResponse {
        let store = require_store!(self, RevokeAgentResponse);
        let principal = require_principal!(RevokeAgentResponse);
        let agent = parse_path_uuid!(RevokeAgentResponse, agent_id, "agent_id");
        let key = parse_idempotency_key!(RevokeAgentResponse, idempotency_key);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            &serde_json::json!({ "reason": body.reason }),
        );
        let ctx = command_context(&principal, key, None, None, store);
        match store
            .revoke_agent(ctx, ActorId::from_uuid(agent), body.reason, &hash)
            .await
        {
            Ok(replay) => RevokeAgentResponse::Ok(wire::Ack {
                ok: replay.into_inner().ok,
            }),
            Err(err) => command_problem!(RevokeAgentResponse, err),
        }
    }

    async fn get_principal(&self) -> GetPrincipalResponse {
        // Credential-derived actor, never caller labels (contract).
        let principal = require_principal!(GetPrincipalResponse);
        GetPrincipalResponse::Ok(wire_actor(&principal))
    }
}
