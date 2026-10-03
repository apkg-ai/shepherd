use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, Response, StatusCode, header};
use axum::middleware::Next;
use shepherd_core::error::DomainError;
use shepherd_core::model::{SESSION_TTL_SECONDS, SecretString, digest_matches, token_digest};
use uuid::Uuid;

use super::context::{CookieAction, RequestContext, with_context};
use super::problem::{problem_body_response, problem_for, problem_response};
use super::{API_PREFIX, LOGIN_PATH, SESSION_COOKIE, SPEC_PATH};
use crate::{AppState, ServiceState};

// ── The gate: request id, Host/Origin, authentication, CSRF, cookies ────

pub async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response<Body> {
    let request_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let is_api = path.starts_with(API_PREFIX);

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
        let ctx = RequestContext::new(request_id, None, None);
        with_context(ctx, next.run(req)).await
    } else {
        api_gate(&state, req, request_id, &path, &method, next).await
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

async fn api_gate(
    state: &AppState,
    req: Request,
    request_id: Uuid,
    path: &str,
    method: &Method,
    next: Next,
) -> Response<Body> {
    let reject = |status: StatusCode, code: &str, detail: &str| {
        problem_response(status, code, detail, request_id)
    };

    let auth_config = &state.auth;
    let is_public = path == SPEC_PATH || (path == LOGIN_PATH && *method == Method::POST);

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

    let ctx = RequestContext::new(request_id, principal, session);
    let mut response = with_context(ctx.clone(), next.run(req)).await;

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
    problem_body_response(status, &body)
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
