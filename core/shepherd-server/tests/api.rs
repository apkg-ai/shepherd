use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use shepherd_server::{AppState, AuthConfig};
use tower::ServiceExt;

// ── Test setup ──────────────────────────────────────────────────────────

const TEST_HOST: &str = "127.0.0.1:7437";

struct Fixture {
    _dir: tempfile::TempDir,
    app: axum::Router,
}

async fn test_app() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = shepherd_core::storage::open(shepherd_core::storage::testing::store_options(
        dir.path(),
        "shepherd.db",
        shepherd_core::storage::testing::test_clock(),
    ))
    .await
    .unwrap();
    let state = AppState::new(Arc::new(store), AuthConfig::new(7437, false));
    Fixture {
        _dir: dir,
        app: shepherd_server::router(state, "does-not-exist"),
    }
}

async fn get_response(app: &axum::Router, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::get(path)
                .header(header::HOST, TEST_HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = get_response(app, path).await;
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

// ── System tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn health_reports_pass_and_core_version() {
    let f = test_app().await;
    let response = get_response(&f.app, "/health").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/health+json"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "pass");
    assert_eq!(json["version"], shepherd_core::version());
    assert_eq!(json["description"], "shepherd local daemon");
}

#[tokio::test]
async fn openapi_spec_is_served_as_yaml() {
    let f = test_app().await;
    let response = get_response(&f.app, "/api/v1/openapi.yaml").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/yaml");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"openapi:"));
}

#[tokio::test]
async fn no_unenforced_rate_limit_headers_are_served() {
    let f = test_app().await;
    // The daemon enforces no quota, so advertising one would be a lie
    // (plan/12 forbids fake RateLimit headers) — on any path.
    for path in ["/health", "/api/v1/principal"] {
        let response = get_response(&f.app, path).await;
        assert!(
            !response.headers().contains_key("RateLimit-Limit"),
            "{path}"
        );
        assert!(
            !response.headers().contains_key("RateLimit-Remaining"),
            "{path}"
        );
        assert!(
            !response.headers().contains_key("RateLimit-Reset"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn cors_allows_exact_local_origins_only() {
    let f = test_app().await;
    let allowed = f
        .app
        .clone()
        .oneshot(
            Request::get("/health")
                .header(header::HOST, TEST_HOST)
                .header("origin", "http://127.0.0.1:7437")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        allowed.headers()["access-control-allow-origin"],
        "http://127.0.0.1:7437"
    );

    // The dev origin needs the explicit --dev flag; any-port loopback is gone.
    for origin in [
        "http://localhost:5173",
        "http://127.0.0.1:9999",
        "https://evil.example.com",
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::get("/health")
                    .header(header::HOST, TEST_HOST)
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin"),
            "{origin} must not be CORS-allowed"
        );
    }
}

// ── Static UI serving ───────────────────────────────────────────────────

#[tokio::test]
async fn untrusted_host_is_rejected_on_every_served_path() {
    let f = test_app().await;
    // /health and the static shell are local-only content (plan/12): an
    // off-allowlist Host must never reach them, rebinding page or not.
    for path in ["/health", "/", "/missing-asset.js"] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "attacker.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        assert!(
            response.headers().contains_key("x-request-id"),
            "{path} must carry X-Request-Id"
        );
        assert!(
            response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY),
            "{path} must carry the CSP"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["code"], "malformed_request", "{path}");
    }

    // The allowlisted Host still reaches every path.
    let response = get_response(&f.app, "/health").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn static_ui_is_served_from_ui_dir() {
    let dir = tempfile::tempdir().unwrap();
    let store = shepherd_core::storage::open(shepherd_core::storage::testing::store_options(
        dir.path(),
        "shepherd.db",
        shepherd_core::storage::testing::test_clock(),
    ))
    .await
    .unwrap();
    let ui_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        ui_dir.path().join("index.html"),
        "<!doctype html><html><body><div id=\"root\"></div></body></html>",
    )
    .unwrap();

    let state = AppState::new(Arc::new(store), AuthConfig::new(7437, false));
    let app = shepherd_server::router(state, ui_dir.path());
    let response = get_response(&app, "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("<div id=\"root\">")
    );

    let response = get_response(&app, "/missing-asset.js").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Scaffold boundary ───────────────────────────────────────────────────

#[tokio::test]
async fn unknown_api_route_requires_authentication() {
    let f = test_app().await;
    // Unauthenticated API paths reject before routing: no route probing.
    for path in ["/api/v1/projects", "/api/v1/events", "/api/v1/anything"] {
        let (status, body) = get(&f.app, path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "expected 401 for {path}");
        assert_eq!(body["code"], "unauthenticated");
    }
}

// ── Contract conformance ────────────────────────────────────────────────

mod contract {
    use super::*;
    use jsonschema_056::Validator;

    fn load_spec() -> Value {
        let yaml_str = include_str!("../../../openapi/shepherd.yaml");
        serde_yaml::from_str(yaml_str).expect("failed to parse OpenAPI spec")
    }

    fn resolve_ref<'a>(spec: &'a Value, ref_path: &str) -> &'a Value {
        let path = ref_path.strip_prefix("#/").unwrap_or(ref_path);
        let mut current = spec;
        for segment in path.split('/') {
            current = &current[segment];
        }
        current
    }

    fn resolve_schema(spec: &Value, schema: &Value) -> Value {
        match schema {
            Value::Object(map) => {
                if let Some(ref_val) = map.get("$ref") {
                    let ref_path = ref_val.as_str().unwrap();
                    let resolved = resolve_ref(spec, ref_path);
                    return resolve_schema(spec, resolved);
                }

                let mut result = serde_json::Map::new();
                for (key, val) in map {
                    result.insert(key.clone(), resolve_schema(spec, val));
                }
                Value::Object(result)
            }
            Value::Array(arr) => {
                Value::Array(arr.iter().map(|v| resolve_schema(spec, v)).collect())
            }
            other => other.clone(),
        }
    }

    fn validate(spec: &Value, schema_ref: &str, value: &Value) {
        let raw = resolve_ref(spec, schema_ref);
        let resolved = resolve_schema(spec, raw);
        let validator = Validator::new(&resolved).expect("failed to compile schema");
        if let Err(err) = validator.validate(value) {
            panic!(
                "Schema validation failed for {schema_ref}:\n  {err}\nValue: {}",
                serde_json::to_string_pretty(value).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn health_response_conforms_to_spec() {
        let spec = load_spec();
        let f = test_app().await;

        let (status, body) = get(&f.app, "/health").await;
        assert_eq!(status, StatusCode::OK);
        validate(&spec, "components/schemas/Health", &body);
    }

    #[tokio::test]
    async fn unauthenticated_problem_conforms_to_spec() {
        let spec = load_spec();
        let f = test_app().await;
        let (status, body) = get(&f.app, "/api/v1/principal").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        validate(&spec, "components/schemas/Problem", &body);
    }

    #[tokio::test]
    async fn catalog_operations_are_not_exposed_without_credentials() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../plan/contracts/operations.json"))
                .expect("operations catalog parses");
        let operations = catalog.as_array().expect("catalog is an array");
        assert_eq!(
            operations.len(),
            70,
            "v1 operation catalog is frozen at step 001"
        );

        let f = test_app().await;
        for op in operations {
            let id = op["operation_id"].as_str().unwrap();
            let method = op["method"].as_str().unwrap().to_uppercase();
            let path = op["path"]
                .as_str()
                .unwrap()
                .split('/')
                .map(|seg| {
                    if seg.starts_with('{') {
                        "00000000-0000-7000-8000-000000000000"
                    } else {
                        seg
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let response = f
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.as_str())
                        .uri(&path)
                        .header(header::HOST, TEST_HOST)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            match id {
                "getHealth" => assert_eq!(status, StatusCode::OK, "health must stay green"),
                // Public login exists but an empty body never succeeds.
                "createBrowserSession" => assert!(
                    status.is_client_error(),
                    "{method} {path} ({id}) must reject an empty login"
                ),
                _ => assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{method} {path} ({id}) must require credentials"
                ),
            }
        }
    }

    #[tokio::test]
    async fn served_spec_is_the_embedded_scaffold_and_identity_contract() {
        let f = test_app().await;
        let response = get_response(&f.app, "/api/v1/openapi.yaml").await;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let served: Value = serde_yaml::from_slice(&bytes).expect("served spec is valid YAML");

        assert_eq!(served, load_spec(), "served spec must be the embedded one");

        let paths = served["paths"].as_object().unwrap();
        // serde_yaml maps sort keys; compare as sets.
        assert_eq!(
            paths.keys().collect::<Vec<_>>(),
            [
                "/api/v1/agents",
                "/api/v1/agents/{agent_id}/revoke",
                "/api/v1/principal",
                "/api/v1/session",
                "/health",
            ]
        );
        assert_eq!(
            served["paths"]["/health"]["get"]["operationId"],
            "getHealth"
        );
        assert_eq!(
            served["paths"]["/api/v1/principal"]["get"]["operationId"],
            "getPrincipal"
        );
    }

    // Step 007 serves exactly the auth-owned operations from the contract
    // (owner-approved deviation from the health-only scaffold; the rest of
    // the surface stays with step 015).
    #[test]
    fn identity_operations_match_the_v1_contract() {
        let spec = load_spec();
        let catalog: Value =
            serde_json::from_str(include_str!("../../../plan/contracts/operations.json"))
                .expect("operations catalog parses");
        let identity_ids = [
            "createBrowserSession",
            "getBrowserSession",
            "deleteBrowserSession",
            "listAgents",
            "createAgent",
            "revokeAgent",
            "getPrincipal",
        ];
        for op in catalog.as_array().unwrap() {
            let id = op["operation_id"].as_str().unwrap();
            if !identity_ids.contains(&id) {
                continue;
            }
            let method = op["method"].as_str().unwrap().to_lowercase();
            let path = op["path"].as_str().unwrap();
            assert_eq!(
                spec["paths"][path][&method]["operationId"], id,
                "{id} must be served at {method} {path}"
            );
        }
    }
}
