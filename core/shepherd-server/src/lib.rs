mod convert;
// The generated module trips these style lints by design.
#[allow(clippy::collapsible_if, clippy::double_must_use)]
pub mod generated;
mod handlers;
mod middleware;

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{any, get};
use shepherd_core::error::DomainError;
use shepherd_core::storage::Store;
use tower_http::services::ServeDir;

use crate::generated::server::router::{
    goals_api_router, identity_api_router, projects_api_router, system_api_router,
    task_types_api_router,
};
pub use crate::middleware::{AuthConfig, REDACTED_HEADERS, redact_header};

pub enum ServiceState {
    Normal(Arc<Store>),
    /// Missing replay key on an existing install (plan/12): health fails,
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

pub(crate) const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

pub fn router(state: AppState, ui_dir: impl AsRef<Path>) -> Router {
    let cors = middleware::cors_layer(&state.auth);
    Router::new()
        // No RateLimit headers anywhere: the daemon enforces no quota, and
        // plan/12 forbids advertising one that is not real.
        .merge(system_api_router(state.clone()))
        .merge(identity_api_router(state.clone()))
        .merge(projects_api_router(state.clone()))
        .merge(goals_api_router(state.clone()))
        .merge(task_types_api_router(state.clone()))
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        // Unrouted API paths answer problem+json, never the static shell.
        .route("/api/{*rest}", any(handlers::api_not_found))
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
