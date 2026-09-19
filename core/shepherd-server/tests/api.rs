use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ── Test setup ──────────────────────────────────────────────────────────

fn test_app() -> axum::Router {
    shepherd_server::router(shepherd_server::AppState, "does-not-exist")
}

async fn get_response(app: &axum::Router, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
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
    let app = test_app();
    let response = get_response(&app, "/health").await;

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
    let app = test_app();
    let response = get_response(&app, "/api/v1/openapi.yaml").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/yaml");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"openapi:"));
}

#[tokio::test]
async fn rate_limit_headers_are_present() {
    let app = test_app();
    let response = get_response(&app, "/health").await;

    assert!(response.headers().contains_key("RateLimit-Limit"));
    assert!(response.headers().contains_key("RateLimit-Remaining"));
    assert!(response.headers().contains_key("RateLimit-Reset"));
}

#[tokio::test]
async fn cors_headers_are_present() {
    let app = test_app();
    let response = app
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );

    let response = app
        .oneshot(
            Request::get("/health")
                .header("origin", "https://evil.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
}

// ── Static UI serving ───────────────────────────────────────────────────

#[tokio::test]
async fn static_ui_is_served_from_ui_dir() {
    let ui_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        ui_dir.path().join("index.html"),
        "<!doctype html><html><body><div id=\"root\"></div></body></html>",
    )
    .unwrap();

    let app = shepherd_server::router(shepherd_server::AppState, ui_dir.path());
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
async fn unknown_api_route_is_not_a_success_stub() {
    let app = test_app();
    for path in ["/api/v1/projects", "/api/v1/events", "/api/v1/anything"] {
        let (status, _) = get(&app, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "expected 404 for {path}");
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
        let app = test_app();

        let (status, body) = get(&app, "/health").await;
        assert_eq!(status, StatusCode::OK);
        validate(&spec, "components/schemas/Health", &body);
    }

    #[tokio::test]
    async fn catalog_operations_are_not_exposed_by_the_scaffold() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../plan/contracts/operations.json"))
                .expect("operations catalog parses");
        let operations = catalog.as_array().expect("catalog is an array");
        assert_eq!(
            operations.len(),
            70,
            "v1 operation catalog is frozen at step 001"
        );

        let app = test_app();
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
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.as_str())
                        .uri(&path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            if id == "getHealth" {
                assert_eq!(status, StatusCode::OK, "health must stay green");
            } else {
                assert!(
                    !status.is_success(),
                    "{method} {path} ({id}) must not succeed on the scaffold"
                );
                // Unrouted: GET falls through to the static dir (404); the
                // static fallback serves no other method (405).
                let expected = if method == "GET" {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::METHOD_NOT_ALLOWED
                };
                assert_eq!(status, expected, "{method} {path} ({id}) must be unrouted");
            }
        }
    }

    #[tokio::test]
    async fn served_spec_is_the_embedded_health_contract() {
        let app = test_app();
        let response = get_response(&app, "/api/v1/openapi.yaml").await;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let served: Value = serde_yaml::from_slice(&bytes).expect("served spec is valid YAML");

        assert_eq!(served, load_spec(), "served spec must be the embedded one");

        let paths = served["paths"].as_object().unwrap();
        assert_eq!(paths.keys().collect::<Vec<_>>(), ["/health"]);
        assert_eq!(
            served["paths"]["/health"]["get"]["operationId"],
            "getHealth"
        );
    }
}
