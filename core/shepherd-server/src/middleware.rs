use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, Response, StatusCode, header};
use axum::middleware::Next;
use http_body_util::BodyExt;
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor as CoreActor, BrowserSession, SESSION_TTL_SECONDS, SecretString, digest_matches,
    token_digest,
};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::generated::types as wire;
use crate::{AppState, ServiceState};

pub const SESSION_COOKIE: &str = "shepherd_session";
const API_PREFIX: &str = "/api/v1";
const SPEC_PATH: &str = "/api/v1/openapi.yaml";
const LOGIN_PATH: &str = "/api/v1/session";

/// Headers whose values must never appear in diagnostics (plan/12).
pub const REDACTED_HEADERS: [&str; 5] = [
    "authorization",
    "cookie",
    "set-cookie",
    "x-csrf-token",
    "x-lease-token",
];

pub fn redact_header(name: &str, value: &str) -> String {
    if REDACTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
        "[redacted]".to_string()
    } else {
        value.to_string()
    }
}

// ── Transport configuration ─────────────────────────────────────────────

pub struct AuthConfig {
    pub port: u16,
    /// Allows the Vite dev origin http://localhost:5173 (plan/12: explicit only).
    pub dev: bool,
}

impl AuthConfig {
    fn allowed_hosts(&self) -> [String; 2] {
        [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ]
    }

    fn allowed_origins(&self) -> Vec<String> {
        let mut origins = vec![
            format!("http://127.0.0.1:{}", self.port),
            format!("http://localhost:{}", self.port),
        ];
        if self.dev {
            origins.push("http://localhost:5173".to_string());
        }
        origins
    }

    // Parse, never prefix-match (plan/12): scheme, host and port must all be
    // exact, which defeats tricks like http://localhost:7437.evil.com.
    fn origin_allowed(&self, origin: &str) -> bool {
        let Ok(parsed) = url::Url::parse(origin) else {
            return false;
        };
        self.allowed_origins().iter().any(|allowed| {
            let expected = url::Url::parse(allowed).expect("allowlist origins parse");
            parsed.scheme() == expected.scheme()
                && parsed.host_str() == expected.host_str()
                && parsed.port_or_known_default() == expected.port_or_known_default()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none()
        })
    }

    fn host_allowed(&self, host: &str) -> bool {
        self.allowed_hosts().iter().any(|allowed| allowed == host)
    }
}

/// Exact-allowlist CORS; credentialed cross-origin only for the dev origin.
pub fn cors_layer(config: &AuthConfig) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .allowed_origins()
        .iter()
        .map(|origin| HeaderValue::from_str(origin).expect("origin is a valid header value"))
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::IF_MATCH,
            "x-csrf-token".parse().unwrap(),
            "idempotency-key".parse().unwrap(),
        ])
        .allow_credentials(config.dev)
}

// ── Request context (task-local) ────────────────────────────────────────

pub enum CookieAction {
    Set(SecretString),
    Clear,
}

pub struct RequestContextInner {
    pub request_id: Uuid,
    pub principal: Option<CoreActor>,
    /// Present only for cookie-authenticated requests.
    pub session: Option<(SecretString, BrowserSession)>,
    pub cookie_action: Mutex<Option<CookieAction>>,
}

#[derive(Clone)]
pub struct RequestContext(pub Arc<RequestContextInner>);

tokio::task_local! {
    static REQUEST_CONTEXT: RequestContext;
}

/// Handlers run inside the gate's task-local scope (generated trait methods
/// cannot receive extractors, so the principal travels here).
pub fn current_context() -> RequestContext {
    REQUEST_CONTEXT.with(Clone::clone)
}

pub fn set_cookie_action(action: CookieAction) {
    let ctx = current_context();
    *ctx.0.cookie_action.lock().unwrap() = Some(action);
}

// ── Problem helpers ─────────────────────────────────────────────────────

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

fn problem_response(
    status: StatusCode,
    code: &str,
    detail: &str,
    request_id: Uuid,
) -> Response<Body> {
    let body =
        serde_json::to_vec(&problem(status, code, detail, request_id)).expect("problem serializes");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/problem+json")
        .body(Body::from(body))
        .expect("problem response builds")
}

// ── The gate: request id, Host/Origin, authentication, CSRF, cookies ────

pub async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response<Body> {
    let request_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let is_api = path.starts_with(API_PREFIX);
    let is_public = path == SPEC_PATH || (path == LOGIN_PATH && method == Method::POST);

    // Exact Host allowlist (plan/12) on every served path — health and
    // static assets included, so a rebinding page cannot read local-only
    // content under an attacker-controlled Host. Parse errors and absence
    // both reject.
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let mut response = if !state.auth.host_allowed(host) {
        problem_response(
            StatusCode::BAD_REQUEST,
            "malformed_request",
            "Host header is not an allowed local host",
            request_id,
        )
    } else if !is_api {
        let ctx = RequestContext(Arc::new(RequestContextInner {
            request_id,
            principal: None,
            session: None,
            cookie_action: Mutex::new(None),
        }));
        REQUEST_CONTEXT.scope(ctx, next.run(req)).await
    } else {
        api_gate(&state, req, request_id, &path, &method, is_public, next).await
    };

    let headers = response.headers_mut();
    headers.insert(
        "X-Request-Id",
        HeaderValue::from_str(&request_id.to_string()).expect("uuid is a valid header value"),
    );
    // plan/12 CSP; style-src unsafe-inline for React Flow inline geometry.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    response
}

#[allow(clippy::too_many_arguments)]
async fn api_gate(
    state: &AppState,
    req: Request,
    request_id: Uuid,
    path: &str,
    method: &Method,
    is_public: bool,
    next: Next,
) -> Response<Body> {
    let reject = |status: StatusCode, code: &str, detail: &str| {
        problem_response(status, code, detail, request_id)
    };

    let auth_config = &state.auth;

    // A supplied Origin must be exactly allowlisted even for bearer clients.
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(origin) = &origin
        && !auth_config.origin_allowed(origin)
    {
        return reject(StatusCode::FORBIDDEN, "forbidden", "Origin is not allowed");
    }

    if path != SPEC_PATH
        && let ServiceState::Diagnostic { reason } = state.service.as_ref()
    {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "integrity_failure", reason);
    }

    // A supplied Authorization header decides the credential entirely: the
    // scheme parses case-insensitively (RFC 7235) and any malformed or
    // unsupported value rejects without ever trying the cookie.
    let authorization = req.headers().get(header::AUTHORIZATION);
    let mut principal = None;
    let mut session = None;
    // OPTIONS carries no credentials: preflights pass (the Host/Origin checks
    // above already screened them) and the inner cors layer answers.
    if !is_public && *method != Method::OPTIONS {
        let ServiceState::Normal(store) = state.service.as_ref() else {
            // Diagnostic mode already rejected above; nothing else reaches here.
            return reject(
                StatusCode::SERVICE_UNAVAILABLE,
                "integrity_failure",
                "service is in diagnostic-only mode",
            );
        };
        if let Some(header_value) = authorization {
            let Some(token) = header_value.to_str().ok().and_then(bearer_token) else {
                return reject(
                    StatusCode::UNAUTHORIZED,
                    "unauthenticated",
                    "Authorization header must be `Bearer <token>`",
                );
            };
            let token = SecretString::new(token.to_string());
            match store.authenticate_bearer(&token).await {
                Ok(actor) => principal = Some(actor),
                Err(err) => return domain_reject(&err, request_id),
            }
        } else if let Some(cookie_token) = session_cookie(req.headers()) {
            let now = store.clock().now();
            match store.browser_session(&cookie_token, now).await {
                Ok(browser_session) => {
                    let mutation = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
                    if mutation {
                        // Cookie mutations need an allowlisted Origin present
                        // and an exact CSRF token (plan/12; login is exempt).
                        if origin.is_none() {
                            return reject(
                                StatusCode::FORBIDDEN,
                                "forbidden",
                                "cookie-authenticated mutations require an allowed Origin",
                            );
                        }
                        let presented = req
                            .headers()
                            .get("X-CSRF-Token")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("");
                        let matches = digest_matches(
                            &token_digest(&SecretString::new(presented.to_string())),
                            &token_digest(&browser_session.csrf_token),
                        );
                        if presented.is_empty() || !matches {
                            return reject(
                                StatusCode::FORBIDDEN,
                                "forbidden",
                                "missing or invalid X-CSRF-Token",
                            );
                        }
                    }
                    principal = Some(browser_session.actor.clone());
                    session = Some((cookie_token, browser_session));
                }
                Err(err) => return domain_reject(&err, request_id),
            }
        } else {
            return reject(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "supply a bearer token or an owner session cookie",
            );
        }
    }

    // Command operations require Idempotency-Key up front (plan/07: 428).
    // Detected here by route so the answer never depends on the generated
    // validator's rejection wording; step 015 generalizes this from the
    // contract catalog.
    if requires_idempotency_key(method, path) && !req.headers().contains_key("Idempotency-Key") {
        return reject(
            StatusCode::PRECONDITION_REQUIRED,
            "precondition_required",
            "supply the Idempotency-Key header",
        );
    }

    let ctx = RequestContext(Arc::new(RequestContextInner {
        request_id,
        principal,
        session,
        cookie_action: Mutex::new(None),
    }));
    let mut response = REQUEST_CONTEXT.scope(ctx.clone(), next.run(req)).await;

    if let Some(action) = ctx.0.cookie_action.lock().unwrap().take() {
        let value = match action {
            // Local HTTP: no Secure attribute (plan/12). Max-Age tracks the
            // server-side session TTL.
            CookieAction::Set(token) => format!(
                "{SESSION_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; \
                 Max-Age={SESSION_TTL_SECONDS}",
                token.expose()
            ),
            CookieAction::Clear => {
                format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
            }
        };
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&value).expect("cookie is a valid header value"),
        );
    }
    response
}

fn domain_reject(err: &DomainError, request_id: Uuid) -> Response<Body> {
    let (status, body) = problem_for(err, request_id);
    let bytes = serde_json::to_vec(&body).expect("problem serializes");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/problem+json")
        .body(Body::from(bytes))
        .expect("problem response builds")
}

// The 007 surface has exactly one replayable command (revokeAgent).
fn requires_idempotency_key(method: &Method, path: &str) -> bool {
    *method == Method::POST
        && path
            .strip_prefix("/api/v1/agents/")
            .and_then(|rest| rest.strip_suffix("/revoke"))
            .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn session_cookie(headers: &axum::http::HeaderMap) -> Option<SecretString> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| SecretString::new(value.to_string()))
    })
}

// ── Problem shaper: contract-shape generated validation rejections ──────

// The generated validation layer emits {type,title,status,code,errors[{code,
// location,message}]} without request_id/detail, and 422 for a missing
// Idempotency-Key where plan/07 requires 428. Reshape inside the gate scope.
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
    let (parts, body) = response.into_parts();
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Response::from_parts(parts, Body::empty()),
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
    fn credential_headers_are_redacted() {
        for name in REDACTED_HEADERS {
            assert_eq!(redact_header(name, "secret-value"), "[redacted]");
        }
        assert_eq!(redact_header("Authorization", "Bearer x"), "[redacted]");
        assert_eq!(
            redact_header("content-type", "application/json"),
            "application/json"
        );
    }

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

    #[test]
    fn origin_allowlist_is_parsed_exactly() {
        let config = AuthConfig {
            port: 7437,
            dev: false,
        };
        assert!(config.origin_allowed("http://127.0.0.1:7437"));
        assert!(config.origin_allowed("http://localhost:7437"));
        assert!(!config.origin_allowed("http://localhost:5173"));
        assert!(!config.origin_allowed("http://localhost:7437.evil.com"));
        assert!(!config.origin_allowed("https://localhost:7437"));
        assert!(!config.origin_allowed("http://user@localhost:7437"));
        assert!(!config.origin_allowed("http://user:pass@localhost:7437"));
        assert!(!config.origin_allowed("http://:pass@localhost:7437"));
        assert!(!config.origin_allowed("http://localhost:7437?x=1"));
        assert!(!config.origin_allowed("http://localhost:7437#frag"));
        let dev = AuthConfig {
            port: 7437,
            dev: true,
        };
        assert!(dev.origin_allowed("http://localhost:5173"));
    }
}
