//! Tower middleware: CORS, stub rate-limit headers, and problem-detail
//! remapping for the generated validation layer.
//!
//! The rate-limit headers satisfy the OpenAPI contract without enforcing
//! actual limits — shepherd is a local daemon.

use axum::body::Body;
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// CORS restricted to loopback origins (any port).
///
/// The UI is served same-origin by this daemon, so cross-origin access only
/// matters for local dev tooling (e.g. a Vite dev server). Restricting the
/// origin prevents arbitrary websites in the user's browser from calling
/// this unauthenticated local API and reading the responses.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            origin.as_bytes().starts_with(b"http://localhost:")
                || origin.as_bytes().starts_with(b"http://127.0.0.1:")
                || origin.as_bytes().starts_with(b"http://[::1]:")
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

// ── Rate-limit header middleware ────────────────────────────────────────

/// Layer that injects stub `RateLimit-*` headers on every response.
#[derive(Clone)]
pub struct RateLimitHeaderLayer;

impl<S> Layer<S> for RateLimitHeaderLayer {
    type Service = RateLimitHeaderService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitHeaderService { inner }
    }
}

/// Service that injects stub rate-limit headers.
#[derive(Clone)]
pub struct RateLimitHeaderService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RateLimitHeaderService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            let mut response = inner.call(req).await?;
            let headers = response.headers_mut();
            headers.insert("RateLimit-Limit", HeaderValue::from_static("1000"));
            headers.insert("RateLimit-Remaining", HeaderValue::from_static("999"));
            headers.insert("RateLimit-Reset", HeaderValue::from_static("60"));
            Ok(response)
        })
    }
}

// ── Problem-detail remap middleware ─────────────────────────────────────

const GENERATOR_PROBLEM_PREFIX: &str = "https://openapi-to-rust.dev/problems/";

/// Layer that rewrites the generated validation layer's problem responses
/// into the shepherd `ProblemDetail` shape from `openapi/shepherd.yaml`.
///
/// `openapi-to-rust` emits its own RFC 9457 profile (`type` under
/// `https://openapi-to-rust.dev/problems/`, a top-level `code`, and
/// `errors[].location`), which violates the spec's `ProblemDetail` schema
/// (`type` must match `^urn:shepherd:error:…`, `additionalProperties: false`,
/// `errors[].field` required). The spec documents only
/// 401/404/409/422/429/500, so every pre-domain rejection — malformed body,
/// bad content type, oversized body, schema violation — is remapped to 422
/// `urn:shepherd:error:validation-error`; internal generator failures map
/// to 500 `urn:shepherd:error:internal-error`. Domain-layer errors already
/// carry shepherd URNs and pass through untouched.
#[derive(Clone)]
pub struct ProblemDetailRemapLayer;

impl<S> Layer<S> for ProblemDetailRemapLayer {
    type Service = ProblemDetailRemapService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ProblemDetailRemapService { inner }
    }
}

/// Service that remaps generator problem responses to the spec shape.
#[derive(Clone)]
pub struct ProblemDetailRemapService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for ProblemDetailRemapService<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            let response = inner.call(req).await?;

            let is_problem = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/problem+json"));
            if !is_problem {
                return Ok(response);
            }

            let (parts, body) = response.into_parts();
            let bytes = match body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(_) => {
                    return Ok(remapped_response(
                        parts,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        internal_error_problem(),
                    ));
                }
            };

            match remap_problem(&bytes) {
                Some((status, value)) => Ok(remapped_response(parts, status, value)),
                None => Ok(Response::from_parts(parts, Body::from(bytes))),
            }
        })
    }
}

/// Rebuild a response with a remapped problem body, dropping the stale
/// `Content-Length` so the framework recomputes it.
fn remapped_response(
    mut parts: axum::http::response::Parts,
    status: StatusCode,
    value: serde_json::Value,
) -> Response<Body> {
    parts.status = status;
    let mut response = Response::from_parts(parts, Body::from(value.to_string()));
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
}

fn internal_error_problem() -> serde_json::Value {
    serde_json::json!({
        "type": "urn:shepherd:error:internal-error",
        "title": "Internal Error",
        "status": 500,
        "detail": "internal server error",
    })
}

/// Map one generator problem body to the shepherd shape.
/// Returns `None` when the body is not a generator problem (pass through).
fn remap_problem(bytes: &[u8]) -> Option<(StatusCode, serde_json::Value)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let problem_type = value.get("type")?.as_str()?;
    let kind = problem_type.strip_prefix(GENERATOR_PROBLEM_PREFIX)?;

    if kind == "generated-contract-error" {
        return Some((StatusCode::INTERNAL_SERVER_ERROR, internal_error_problem()));
    }

    let detail = match kind {
        "validation" => "One or more request fields failed validation.",
        "malformed-request" => "The request body is malformed.",
        "malformed-parameter" => "A request parameter is malformed.",
        "request-body-too-large" => "The request body exceeds the size limit.",
        "unsupported-media-type" => "The request content type is not supported.",
        _ => "The request failed validation.",
    };

    let errors: Vec<serde_json::Value> = value
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let field = item.get("location")?.as_str()?;
                    let message = item.get("message")?.as_str()?;
                    let mut mapped = serde_json::json!({
                        "field": field,
                        "message": message,
                    });
                    if let Some(code) = item.get("code").and_then(|c| c.as_str()) {
                        mapped["code"] = serde_json::json!(code);
                    }
                    Some(mapped)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut problem = serde_json::json!({
        "type": "urn:shepherd:error:validation-error",
        "title": "Validation Error",
        "status": 422,
        "detail": detail,
    });
    if !errors.is_empty() {
        problem["errors"] = serde_json::json!(errors);
    }
    Some((StatusCode::UNPROCESSABLE_ENTITY, problem))
}
