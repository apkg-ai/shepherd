use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shepherd_core::commands::IdentityPaths;
use shepherd_core::model::TestClock;
use shepherd_core::storage::{Store, open, testing};
use shepherd_server::{AppState, AuthConfig};
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:7437";
const ORIGIN: &str = "http://127.0.0.1:7437";

struct Fixture {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Arc<Store>,
    app: axum::Router,
    owner_token: String,
}

async fn fixture_with(dev: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = Arc::new(
        open(testing::store_options(
            dir.path(),
            "shepherd.db",
            clock.clone(),
        ))
        .await
        .unwrap(),
    );
    store
        .ensure_owner(&IdentityPaths::new(dir.path()))
        .await
        .unwrap();
    let owner_token = std::fs::read_to_string(dir.path().join("owner-token")).unwrap();
    let state = AppState::new(store.clone(), AuthConfig { port: 7437, dev });
    let app = shepherd_server::router(state, "does-not-exist");
    Fixture {
        _dir: dir,
        clock,
        store,
        app,
        owner_token,
    }
}

async fn fixture() -> Fixture {
    fixture_with(false).await
}

struct TestRequest {
    builder: axum::http::request::Builder,
    body: Body,
}

fn req(method: Method, path: &str) -> TestRequest {
    TestRequest {
        builder: Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST),
        body: Body::empty(),
    }
}

impl TestRequest {
    fn header(mut self, name: &str, value: &str) -> Self {
        self.builder = self.builder.header(name, value);
        self
    }

    fn json(mut self, value: Value) -> Self {
        self.builder = self
            .builder
            .header(header::CONTENT_TYPE, "application/json");
        self.body = Body::from(serde_json::to_vec(&value).unwrap());
        self
    }

    async fn send(self, app: &axum::Router) -> (StatusCode, axum::http::HeaderMap, Value) {
        let response = app
            .clone()
            .oneshot(self.builder.body(self.body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, headers, json)
    }
}

struct OwnerSession {
    cookie: String,
    csrf: String,
}

async fn login(f: &Fixture) -> OwnerSession {
    let (status, headers, body) = req(Method::POST, "/api/v1/session")
        .header("origin", ORIGIN)
        .json(json!({ "owner_token": f.owner_token }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let set_cookie = headers[header::SET_COOKIE].to_str().unwrap().to_string();
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(set_cookie.contains("SameSite=Strict"), "{set_cookie}");
    assert!(set_cookie.contains("Max-Age=43200"), "{set_cookie}");
    assert!(
        !set_cookie.contains("Secure"),
        "loopback HTTP: {set_cookie}"
    );
    let cookie = set_cookie.split(';').next().unwrap().to_string();
    OwnerSession {
        cookie,
        csrf: body["csrf_token"].as_str().unwrap().to_string(),
    }
}

async fn create_agent(f: &Fixture, session: &OwnerSession, label: &str) -> (String, String) {
    let (status, _, body) = req(Method::POST, "/api/v1/agents")
        .header("origin", ORIGIN)
        .header("cookie", &session.cookie)
        .header("X-CSRF-Token", &session.csrf)
        .json(json!({ "label": label }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["actor"]["id"].as_str().unwrap().to_string(),
        body["token"].as_str().unwrap().to_string(),
    )
}

async fn actor_count(f: &Fixture) -> usize {
    f.store
        .list_agents(&shepherd_core::queries::ListParams::default())
        .await
        .unwrap()
        .items
        .len()
}

// ── Lifecycle ───────────────────────────────────────────────────────────

#[tokio::test]
async fn full_lifecycle_login_issue_restrict_revoke_logout() {
    let f = fixture().await;
    let session = login(&f).await;

    // GET /session re-serves the same CSRF and fixed expiry.
    let (status, _, body) = req(Method::GET, "/api/v1/session")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["csrf_token"], session.csrf.as_str());
    assert_eq!(body["actor"]["kind"], "human");

    let (agent_id, agent_token) = create_agent(&f, &session, "builder").await;

    // The agent's principal is credential-derived.
    let (status, _, body) = req(Method::GET, "/api/v1/principal")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "agent");
    assert_eq!(body["id"], agent_id.as_str());

    // Agents cannot mint agents.
    let (status, _, body) = req(Method::POST, "/api/v1/agents")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .json(json!({ "label": "minion" }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "forbidden");

    // Owner lists agents; agent may not.
    let (status, _, body) = req(Method::GET, "/api/v1/agents")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (status, _, _) = req(Method::GET, "/api/v1/agents")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Revoke with idempotency key; replay returns the same answer.
    let key = "11111111-2222-7333-8444-555555555555";
    for round in 0..2 {
        let (status, _, body) = req(Method::POST, &format!("/api/v1/agents/{agent_id}/revoke"))
            .header("origin", ORIGIN)
            .header("cookie", &session.cookie)
            .header("X-CSRF-Token", &session.csrf)
            .header("Idempotency-Key", key)
            .json(json!({ "reason": "went rogue" }))
            .send(&f.app)
            .await;
        assert_eq!(status, StatusCode::OK, "round {round}: {body}");
        assert_eq!(body["ok"], true, "round {round}");
    }

    // Revoked token fails everywhere.
    let (status, _, body) = req(Method::GET, "/api/v1/principal")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthenticated");

    // Logout deletes the session and expires the cookie.
    let (status, headers, body) = req(Method::DELETE, "/api/v1/session")
        .header("origin", ORIGIN)
        .header("cookie", &session.cookie)
        .header("X-CSRF-Token", &session.csrf)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert!(
        headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let (status, _, _) = req(Method::GET, "/api/v1/session")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ── Acceptance: forged roles ────────────────────────────────────────────

#[tokio::test]
async fn forged_owner_labels_never_grant_rights() {
    let f = fixture().await;
    let session = login(&f).await;
    let (_, agent_token) = create_agent(&f, &session, "builder").await;
    let before = actor_count(&f).await;

    // Extra body fields claiming a role are rejected by the contract itself.
    let (status, _, body) = req(Method::POST, "/api/v1/agents")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .json(json!({ "label": "minion", "kind": "human", "role": "owner" }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["request_id"].is_string(), "shaped problem: {body}");

    // A clean body still hits the capability matrix.
    let (status, _, _) = req(Method::POST, "/api/v1/agents")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .json(json!({ "label": "minion" }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(actor_count(&f).await, before);
}

// ── Acceptance: bad Origin / Host / CSRF ────────────────────────────────

#[tokio::test]
async fn cookie_mutation_requires_allowlisted_origin() {
    let f = fixture().await;
    let session = login(&f).await;
    let before = actor_count(&f).await;
    for origin in [None, Some("https://evil.example.com")] {
        let mut request = req(Method::POST, "/api/v1/agents")
            .header("cookie", &session.cookie)
            .header("X-CSRF-Token", &session.csrf);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let (status, _, body) = request.json(json!({ "label": "x" })).send(&f.app).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "origin {origin:?}: {body}");
        assert_eq!(body["code"], "forbidden");
    }
    assert_eq!(actor_count(&f).await, before);
}

#[tokio::test]
async fn csrf_token_required_and_exact() {
    let f = fixture().await;
    let session = login(&f).await;
    let before = actor_count(&f).await;
    for csrf in [None, Some("wrong-token")] {
        let mut request = req(Method::POST, "/api/v1/agents")
            .header("origin", ORIGIN)
            .header("cookie", &session.cookie);
        if let Some(csrf) = csrf {
            request = request.header("X-CSRF-Token", csrf);
        }
        let (status, _, body) = request.json(json!({ "label": "x" })).send(&f.app).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "csrf {csrf:?}: {body}");
    }
    assert_eq!(actor_count(&f).await, before);
}

#[tokio::test]
async fn host_header_must_match_allowlist() {
    let f = fixture().await;
    for host in ["evil.example.com:7437", "127.0.0.1:9999", "localhost"] {
        let (status, _, body) = TestRequest {
            builder: Request::builder()
                .method(Method::GET)
                .uri("/api/v1/principal")
                .header(header::HOST, host)
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", f.owner_token.clone()),
                ),
            body: Body::empty(),
        }
        .send(&f.app)
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "host {host}: {body}");
        assert_eq!(body["code"], "malformed_request");
    }
}

#[tokio::test]
async fn origin_prefix_tricks_are_rejected() {
    let f = fixture().await;
    // Valid bearer, hostile Origin: URL-parsed exactness, never starts_with.
    for origin in [
        "http://localhost:7437.evil.com",
        "http://evil.com/http://localhost:7437",
        "http://user@localhost:7437",
        "http://localhost:7437/path",
        "https://localhost:7437",
        "not a url",
    ] {
        let (status, _, body) = req(Method::GET, "/api/v1/principal")
            .header(
                header::AUTHORIZATION.as_str(),
                &format!("Bearer {}", f.owner_token),
            )
            .header("origin", origin)
            .send(&f.app)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "origin {origin}: {body}");
    }
}

#[tokio::test]
async fn invalid_bearer_never_falls_back_to_cookie() {
    let f = fixture().await;
    let session = login(&f).await;
    let (status, _, body) = req(Method::GET, "/api/v1/principal")
        .header(header::AUTHORIZATION.as_str(), "Bearer not-a-real-token")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn bearer_wins_over_cookie() {
    let f = fixture().await;
    let session = login(&f).await;
    let (_, agent_token) = create_agent(&f, &session, "builder").await;
    // Both credentials supplied: the bearer identity (agent) answers.
    let (status, _, body) = req(Method::GET, "/api/v1/principal")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {agent_token}"),
        )
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "agent");

    // A bearer caller has no browser-session resource.
    let (status, _, _) = req(Method::GET, "/api/v1/session")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {}", f.owner_token),
        )
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dev_origin_only_with_dev_flag() {
    let f = fixture().await;
    let (status, _, _) = req(Method::GET, "/api/v1/principal")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {}", f.owner_token),
        )
        .header("origin", "http://localhost:5173")
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let dev = fixture_with(true).await;
    let (status, _, _) = req(Method::GET, "/api/v1/principal")
        .header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {}", dev.owner_token),
        )
        .header("origin", "http://localhost:5173")
        .send(&dev.app)
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ── Problem shaping ─────────────────────────────────────────────────────

#[tokio::test]
async fn missing_idempotency_key_is_precondition_required() {
    let f = fixture().await;
    let session = login(&f).await;
    let (agent_id, _) = create_agent(&f, &session, "builder").await;
    let (status, _, body) = req(Method::POST, &format!("/api/v1/agents/{agent_id}/revoke"))
        .header("origin", ORIGIN)
        .header("cookie", &session.cookie)
        .header("X-CSRF-Token", &session.csrf)
        .json(json!({ "reason": "no key" }))
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["code"], "precondition_required");
    assert!(body["request_id"].is_string(), "{body}");
}

#[tokio::test]
async fn problems_carry_request_id_and_responses_carry_security_headers() {
    let f = fixture().await;
    let (status, headers, body) = req(Method::GET, "/api/v1/principal").send(&f.app).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let header_id = headers["x-request-id"].to_str().unwrap();
    assert_eq!(body["request_id"], header_id);
    assert_eq!(body["type"], "urn:shepherd:error:unauthenticated");
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");

    // Every response carries a request id, success included.
    let (_, headers, _) = req(Method::GET, "/health").send(&f.app).await;
    assert!(headers.contains_key("x-request-id"));
}

// ── Diagnostic-only mode ────────────────────────────────────────────────

#[tokio::test]
async fn missing_replay_key_enters_diagnostic_only_mode() {
    let state = AppState::diagnostic(
        "replay key /tmp/replay-key is missing".to_string(),
        AuthConfig {
            port: 7437,
            dev: false,
        },
    );
    let app = shepherd_server::router(state, "does-not-exist");

    let (status, _, body) = req(Method::GET, "/health").send(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "warn");
    assert!(body["output"].as_str().unwrap().contains("replay key"));

    let (status, _, body) = req(Method::GET, "/api/v1/principal").send(&app).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "integrity_failure");

    // The spec stays readable for diagnostics.
    let (status, _, _) = req(Method::GET, "/api/v1/openapi.yaml").send(&app).await;
    assert_eq!(status, StatusCode::OK);
}

// ── Task-local integrity under concurrency ──────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_context_survives_concurrent_requests() {
    let f = fixture().await;
    let session = login(&f).await;
    let mut agents = Vec::new();
    for label in ["a", "b", "c", "d", "e"] {
        f.clock.advance(chrono::TimeDelta::seconds(1));
        agents.push(create_agent(&f, &session, label).await);
    }
    let mut handles = Vec::new();
    for (id, token) in agents {
        let app = f.app.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                let (status, _, body) = req(Method::GET, "/api/v1/principal")
                    .header(header::AUTHORIZATION.as_str(), &format!("Bearer {token}"))
                    .send(&app)
                    .await;
                assert_eq!(status, StatusCode::OK);
                // Each request sees exactly its own principal.
                assert_eq!(body["id"], id.as_str());
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
}

// ── Session TTL at the HTTP boundary ────────────────────────────────────

#[tokio::test]
async fn browser_session_expiry_is_fixed_at_the_http_boundary() {
    let f = fixture().await;
    let session = login(&f).await;
    f.clock.advance(chrono::TimeDelta::hours(11));
    let (status, _, first) = req(Method::GET, "/api/v1/session")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    f.clock.advance(chrono::TimeDelta::minutes(30));
    let (status, _, second) = req(Method::GET, "/api/v1/session")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["expires_at"], second["expires_at"], "no sliding");
    f.clock.advance(chrono::TimeDelta::minutes(31));
    let (status, _, _) = req(Method::GET, "/api/v1/session")
        .header("cookie", &session.cookie)
        .send(&f.app)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
