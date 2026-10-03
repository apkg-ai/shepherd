use axum::body::Body;
use axum::extract::Request;
use axum::http::{Response, StatusCode, header};
use axum::middleware::Next;
use http_body_util::BodyExt;
use shepherd_core::error::DomainError;
use uuid::Uuid;

use super::{API_PREFIX, current_context};
use crate::generated::types as wire;

pub fn problem(status: StatusCode, code: &str, detail: &str, request_id: Uuid) -> wire::Problem {
    wire::Problem {
        r#type: format!("urn:shepherd:error:{code}"),
        title: status.canonical_reason().unwrap_or("Error").to_string(),
        status: i32::from(status.as_u16()),
        code: code.to_string(),
        detail: detail.to_string(),
        request_id,
        errors: None,
    }
}

/// Domain error → (status, contract Problem) per the plan/07 table.
pub fn problem_for(err: &DomainError, request_id: Uuid) -> (StatusCode, wire::Problem) {
    let status = match err.code() {
        "unauthenticated" => StatusCode::UNAUTHORIZED,
        "forbidden" => StatusCode::FORBIDDEN,
        "not_found" => StatusCode::NOT_FOUND,
        // invalid_state joins the workflow-conflict family (see error.rs).
        "idempotency_conflict"
        | "terminal"
        | "active_work"
        | "dependency_cycle"
        | "scope_mismatch"
        | "invalid_state" => StatusCode::CONFLICT,
        "revision_conflict" => StatusCode::PRECONDITION_FAILED,
        // graph_too_large is 422 per plan/05 (recorded in the step-005 handoff).
        "validation_error" | "invalid_cursor" | "graph_too_large" => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        "precondition_required" => StatusCode::PRECONDITION_REQUIRED,
        "storage_busy" | "integrity_failure" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    // Storage details never reach the wire (plan/07: no SQL or filesystem secrets).
    let detail = match err {
        DomainError::Storage(_) => "internal storage error".to_string(),
        other => other.to_string(),
    };
    (status, problem(status, err.code(), &detail, request_id))
}

pub(crate) fn problem_response(
    status: StatusCode,
    code: &str,
    detail: &str,
    request_id: Uuid,
) -> Response<Body> {
    problem_body_response(status, &problem(status, code, detail, request_id))
}

pub(crate) fn problem_body_response(status: StatusCode, body: &wire::Problem) -> Response<Body> {
    let bytes = serde_json::to_vec(body).expect("problem serializes");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/problem+json")
        .body(Body::from(bytes))
        .expect("problem response builds")
}

// The generated validation layer emits {type,title,status,code,errors[{code,
// location,message}]} without request_id/detail. Reshape inside the gate scope.
pub async fn problem_shaper(req: Request, next: Next) -> Response<Body> {
    let is_api = req.uri().path().starts_with(API_PREFIX);
    let response = next.run(req).await;
    if !is_api || !response.status().is_client_error() {
        return response;
    }
    let is_problem = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/problem+json"));
    if !is_problem {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            // The inner body is gone; a stale Content-Length would make the
            // empty replacement unreadable at the HTTP framing layer.
            parts.headers.remove(header::CONTENT_LENGTH);
            return Response::from_parts(parts, Body::empty());
        }
    };
    let Ok(rejection) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    if rejection.get("request_id").is_some() {
        // Already contract-shaped (one of ours).
        return Response::from_parts(parts, Body::from(bytes));
    }
    let request_id = current_context().0.request_id;
    let errors: Vec<wire::ProblemErrorsItem> = rejection["errors"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| wire::ProblemErrorsItem {
                    field: item["location"].as_str().unwrap_or("").to_string(),
                    message: item["message"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let status = parts.status;
    let code = match status {
        StatusCode::BAD_REQUEST => "malformed_request",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        _ => "validation_error",
    };
    let detail = errors
        .first()
        .map(|item| format!("{} {}", item.field, item.message))
        .unwrap_or_else(|| "request validation failed".to_string());
    let mut shaped = problem(status, code, &detail, request_id);
    if !errors.is_empty() {
        shaped.errors = Some(errors);
    }
    let bytes = serde_json::to_vec(&shaped).expect("problem serializes");
    let mut response = Response::from_parts(parts, Body::from(bytes));
    *response.status_mut() = status;
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_for_maps_every_domain_code_to_its_plan07_status() {
        use shepherd_core::error::DomainError;
        use shepherd_core::storage::StorageError;
        let request_id = Uuid::nil();
        let cases: Vec<(DomainError, StatusCode)> = vec![
            (DomainError::NotFound, StatusCode::NOT_FOUND),
            (DomainError::Unauthenticated, StatusCode::UNAUTHORIZED),
            (DomainError::Forbidden("x".into()), StatusCode::FORBIDDEN),
            (DomainError::IdempotencyConflict, StatusCode::CONFLICT),
            (
                DomainError::RevisionConflict {
                    expected: 2,
                    actual: 1,
                },
                StatusCode::PRECONDITION_FAILED,
            ),
            (
                DomainError::PreconditionRequired,
                StatusCode::PRECONDITION_REQUIRED,
            ),
            (
                DomainError::Validation {
                    field: "label",
                    message: "blank".into(),
                },
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                DomainError::InvalidCursor("bad".into()),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                DomainError::DuplicateTaskTypeKey { key: "code".into() },
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (DomainError::ArchivedScope, StatusCode::CONFLICT),
            (DomainError::TerminalScope, StatusCode::CONFLICT),
            (
                DomainError::ActiveWork("claimed".into()),
                StatusCode::CONFLICT,
            ),
            (DomainError::DependencyCycle, StatusCode::CONFLICT),
            (
                DomainError::ScopeMismatch("cross".into()),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::GraphTooLarge {
                    nodes: 2001,
                    dependencies: 0,
                },
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                DomainError::InvalidState("wrong".into()),
                StatusCode::CONFLICT,
            ),
            (
                DomainError::Storage(StorageError::Corrupt("bad".into())),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                DomainError::Storage(StorageError::Codec("boom".into())),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (err, expected) in cases {
            let (status, body) = problem_for(&err, request_id);
            assert_eq!(status, expected, "{}", err.code());
            assert_eq!(body.status, i32::from(expected.as_u16()), "{}", err.code());
            assert_eq!(body.code, err.code());
            assert_eq!(body.r#type, format!("urn:shepherd:error:{}", err.code()));
        }
        // Storage details never reach the wire.
        let (_, body) = problem_for(
            &DomainError::Storage(StorageError::Codec("secret path".into())),
            request_id,
        );
        assert_eq!(body.detail, "internal storage error");
    }
}
