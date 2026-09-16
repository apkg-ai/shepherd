// The generated module trips these style lints by design.
#[allow(clippy::collapsible_if, clippy::double_must_use)]
pub mod generated;
mod middleware;

use std::path::Path;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use tower_http::services::ServeDir;

use crate::generated::server::api::SystemApi;
use crate::generated::server::errors::GetHealthResponse;
use crate::generated::server::router::system_api_router;
use crate::generated::types as wire;
use crate::middleware::{RateLimitHeaderLayer, cors_layer};

#[derive(Clone)]
pub struct AppState;

const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

pub fn router(state: AppState, ui_dir: impl AsRef<Path>) -> Router {
    system_api_router(state)
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        // The fallback must precede the layers — Router::layer wraps only what comes before it.
        .fallback_service(ServeDir::new(ui_dir.as_ref()))
        .layer(RateLimitHeaderLayer)
        .layer(cors_layer())
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}

// ── SystemApi ───────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl SystemApi for AppState {
    async fn get_health(&self) -> GetHealthResponse {
        GetHealthResponse::Ok(wire::Health {
            status: wire::HealthStatus::Pass,
            version: shepherd_core::version().to_string(),
            description: Some("shepherd local daemon".to_string()),
            output: None,
            notes: None,
        })
    }
}
