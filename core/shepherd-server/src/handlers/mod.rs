pub(crate) mod claims;
pub(crate) mod content;
pub(crate) mod events;
pub(crate) mod hierarchy;
pub(crate) mod identity;
pub(crate) mod reviews;

use axum::body::Body;
use axum::http::{Response, StatusCode};

use crate::generated::server::api::SystemApi;
use crate::generated::server::errors::GetHealthResponse;
use crate::generated::types as wire;
use crate::middleware::problem_body_response;
use crate::{AppState, ServiceState, convert};

#[async_trait::async_trait]
impl SystemApi for AppState {
    async fn get_health(&self) -> GetHealthResponse {
        // Diagnostic mode is unhealthy for API purposes (plan/12); the reason
        // stays in logs, the stable contract carries no free-text field.
        let status = match self.service.as_ref() {
            ServiceState::Normal(_) => wire::HealthStatus::Pass,
            ServiceState::Diagnostic { .. } => wire::HealthStatus::Fail,
        };
        GetHealthResponse::Ok(wire::Health {
            status,
            version: shepherd_core::version().to_string(),
            schema_version: shepherd_core::storage::SCHEMA_VERSION as i32,
        })
    }
}

/// Truthful problem+json 404 for unrouted API paths — the deferred portability
/// operations included — instead of the static-UI fallback.
pub(crate) async fn api_not_found() -> Response<Body> {
    let request_id = convert::request_id();
    problem_body_response(
        StatusCode::NOT_FOUND,
        &crate::middleware::problem(
            StatusCode::NOT_FOUND,
            "not_found",
            "unknown API route",
            request_id,
        ),
    )
}
