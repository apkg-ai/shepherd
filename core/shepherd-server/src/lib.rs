//! Router and handlers for the shepherd daemon, exposed as a lib target so
//! integration tests (`tests/`) can drive them without booting the binary.
//! Keep this thin: logic belongs in `shepherd-core`.

use std::path::Path;

use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tower_http::services::ServeDir;

/// The served contract. Embedded so the binary is self-contained.
const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

/// Build the daemon's router: `/health`, the served spec, and the built UI.
pub fn router(ui_dir: impl AsRef<Path>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        .fallback_service(ServeDir::new(ui_dir.as_ref()))
}

/// Health check per draft-inadarei-api-health-check-06.
async fn health() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/health+json")],
        Json(serde_json::json!({
            "status": "pass",
            "version": shepherd_core::version(),
            "description": "shepherd local daemon",
        })),
    )
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}
