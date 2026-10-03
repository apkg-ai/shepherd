// The generated module trips these style lints by design.
#[allow(clippy::collapsible_if, clippy::double_must_use)]
pub mod generated;
mod middleware;

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use shepherd_core::commands::CommandContext;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor as CoreActor, ActorId, ActorKind, AgentCreate as CoreAgentCreate, CommandId,
    SecretString, canonical_request_hash,
};
use shepherd_core::queries::ListParams;
use shepherd_core::storage::Store;
use tower_http::services::ServeDir;
use uuid::Uuid;

use crate::generated::server::api::{IdentityApi, SystemApi};
use crate::generated::server::errors::{
    CreateAgentResponse, CreateBrowserSessionResponse, DeleteBrowserSessionResponse,
    GetBrowserSessionResponse, GetHealthResponse, GetPrincipalResponse, ListAgentsResponse,
    RevokeAgentResponse,
};
use crate::generated::server::router::{identity_api_router, system_api_router};
use crate::generated::types as wire;
pub use crate::middleware::{AuthConfig, REDACTED_HEADERS, redact_header};
use crate::middleware::{CookieAction, current_context, problem, problem_for, set_cookie_action};

pub enum ServiceState {
    Normal(Arc<Store>),
    /// Missing replay key on an existing install (plan/12): health warns,
    /// every API operation returns 503 integrity_failure.
    Diagnostic {
        reason: String,
    },
}

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<ServiceState>,
    pub auth: Arc<AuthConfig>,
}

impl AppState {
    pub fn new(store: Arc<Store>, auth: AuthConfig) -> Self {
        Self {
            service: Arc::new(ServiceState::Normal(store)),
            auth: Arc::new(auth),
        }
    }

    pub fn diagnostic(reason: String, auth: AuthConfig) -> Self {
        Self {
            service: Arc::new(ServiceState::Diagnostic { reason }),
            auth: Arc::new(auth),
        }
    }

    fn store(&self) -> Result<&Store, DomainError> {
        match self.service.as_ref() {
            ServiceState::Normal(store) => Ok(store),
            ServiceState::Diagnostic { .. } => Err(DomainError::Storage(
                shepherd_core::storage::StorageError::Corrupt(
                    "service is in diagnostic-only mode".into(),
                ),
            )),
        }
    }
}

const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

pub fn router(state: AppState, ui_dir: impl AsRef<Path>) -> Router {
    let cors = middleware::cors_layer(&state.auth);
    Router::new()
        // No RateLimit headers anywhere: the daemon enforces no quota, and
        // plan/12 forbids advertising one that is not real.
        .merge(system_api_router(state.clone()))
        .merge(identity_api_router(state.clone()))
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        // The fallback must precede the layers — Router::layer wraps only what comes before it.
        .fallback_service(ServeDir::new(ui_dir.as_ref()))
        .layer(axum::middleware::from_fn(middleware::problem_shaper))
        // The gate must be outermost so CORS preflights pass its Host/Origin
        // checks and carry X-Request-Id/CSP too; cors answers them from inside.
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::gate,
        ))
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}

// ── SystemApi ───────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl SystemApi for AppState {
    async fn get_health(&self) -> GetHealthResponse {
        match self.service.as_ref() {
            ServiceState::Normal(_) => GetHealthResponse::Ok(wire::Health {
                status: wire::HealthStatus::Pass,
                version: shepherd_core::version().to_string(),
                description: Some("shepherd local daemon".to_string()),
                output: None,
                notes: None,
            }),
            ServiceState::Diagnostic { reason } => GetHealthResponse::Ok(wire::Health {
                status: wire::HealthStatus::Warn,
                version: shepherd_core::version().to_string(),
                description: Some("shepherd local daemon".to_string()),
                output: Some(reason.clone()),
                notes: None,
            }),
        }
    }
}

// ── IdentityApi ─────────────────────────────────────────────────────────

fn wire_actor(actor: &CoreActor) -> wire::Actor {
    wire::Actor {
        id: actor.id.as_uuid(),
        kind: match actor.kind {
            ActorKind::Human => wire::ActorKind::Human,
            ActorKind::Agent => wire::ActorKind::Agent,
        },
        label: actor.label.clone(),
        revoked: actor.revoked,
    }
}

fn command_context(actor: &CoreActor, idempotency_key: Uuid, store: &Store) -> CommandContext {
    let now = store.clock().now();
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key,
        expected_revision: None,
        now,
    }
}

fn request_id() -> Uuid {
    current_context().0.request_id
}

// The identity response enums share these variant names; map the plan/07
// status for a domain error onto the right variant.
macro_rules! domain_problem {
    ($response:ident, $err:expr) => {{
        let (status, body) = problem_for(&$err, request_id());
        match status {
            StatusCode::BAD_REQUEST => $response::BadRequest(body),
            StatusCode::UNAUTHORIZED => $response::Unauthorized(body),
            StatusCode::FORBIDDEN => $response::Forbidden(body),
            StatusCode::NOT_FOUND => $response::NotFound(body),
            StatusCode::CONFLICT => $response::Conflict(body),
            StatusCode::UNPROCESSABLE_ENTITY => $response::UnprocessableEntity(body),
            StatusCode::SERVICE_UNAVAILABLE => $response::ServiceUnavailable(body),
            // A status the operation's contract does not declare (e.g. 412/428
            // on a read): re-coerce the body so wire status and body agree
            // instead of shipping a 500 that claims another status.
            _ => $response::InternalServerError(problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                &body.detail,
                request_id(),
            )),
        }
    }};
}

// Cookie-only session endpoints return 404 for bearer callers: a bearer
// client has no browser session resource.
macro_rules! require_session {
    ($response:ident) => {{
        let ctx = current_context();
        match &ctx.0.session {
            Some(session) => (session.0.clone(), session.1.actor.clone()),
            None => {
                return $response::NotFound(problem(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "no browser session for this credential",
                    request_id(),
                ));
            }
        }
    }};
}

macro_rules! require_principal {
    ($response:ident) => {{
        match current_context().0.principal.clone() {
            Some(principal) => principal,
            None => {
                return $response::Unauthorized(problem(
                    StatusCode::UNAUTHORIZED,
                    "unauthenticated",
                    "authentication required",
                    request_id(),
                ));
            }
        }
    }};
}

macro_rules! require_store {
    ($self:ident, $response:ident) => {{
        match $self.store() {
            Ok(store) => store,
            Err(err) => return domain_problem!($response, err),
        }
    }};
}

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
        let params = ListParams {
            // Negatives are unreachable past the generated validation
            // (minimum: 1); 0 falls to effective_limit's range error.
            limit: limit.map(|value| u32::try_from(value).unwrap_or(0)),
            cursor,
            include_archived: false,
        };
        match store.list_agents(&principal.id, &params).await {
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
        let Ok(agent) = agent_id.parse::<Uuid>() else {
            return RevokeAgentResponse::UnprocessableEntity(problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "agent_id must be a UUID",
                request_id(),
            ));
        };
        let Ok(key) = idempotency_key.parse::<Uuid>() else {
            return RevokeAgentResponse::BadRequest(problem(
                StatusCode::BAD_REQUEST,
                "malformed_request",
                "Idempotency-Key must be a UUID",
                request_id(),
            ));
        };
        let request_hash = canonical_request_hash(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            &serde_json::json!({ "reason": body.reason }),
        );
        let ctx = command_context(&principal, key, store);
        match store
            .revoke_agent(ctx, ActorId::from_uuid(agent), body.reason, &request_hash)
            .await
        {
            Ok(replay) => RevokeAgentResponse::Ok(wire::Ack {
                ok: replay.into_inner().ok,
            }),
            // revokeAgent's contract declares 428 (missing command header),
            // which the shared macro cannot name for the other operations.
            Err(err) => {
                let (status, body) = problem_for(&err, request_id());
                if status == StatusCode::PRECONDITION_REQUIRED {
                    return RevokeAgentResponse::Status428(body);
                }
                domain_problem!(RevokeAgentResponse, err)
            }
        }
    }

    async fn get_principal(&self) -> GetPrincipalResponse {
        // Credential-derived actor, never caller labels (contract).
        let principal = require_principal!(GetPrincipalResponse);
        GetPrincipalResponse::Ok(wire_actor(&principal))
    }
}
