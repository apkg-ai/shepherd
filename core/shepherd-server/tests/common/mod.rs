// Shared REST test world; each integration binary uses its own subset.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use shepherd_core::commands::IdentityPaths;
use shepherd_core::model::TestClock;
use shepherd_core::storage::{Store, open, testing};
use shepherd_server::{AppState, AuthConfig};
use tower::ServiceExt;

pub const HOST: &str = "127.0.0.1:7437";

pub struct World {
    pub _dir: tempfile::TempDir,
    pub clock: Arc<TestClock>,
    pub store: Arc<Store>,
    pub app: axum::Router,
    pub owner_token: String,
}

pub async fn world() -> World {
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
    let state = AppState::new(store.clone(), AuthConfig::new(7437, false));
    let app = shepherd_server::router(state, "does-not-exist");
    World {
        _dir: dir,
        clock,
        store,
        app,
        owner_token,
    }
}

impl World {
    /// Owner-authenticated request with the Host header preset.
    pub fn owner(&self, method: Method, path: &str) -> TestRequest {
        self.req(method, path).header(
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {}", self.owner_token),
        )
    }

    pub fn req(&self, method: Method, path: &str) -> TestRequest {
        TestRequest {
            builder: Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, HOST),
            body: Body::empty(),
        }
    }

    /// Issues a fresh agent bearer token through the public command surface.
    pub async fn agent_token(&self, label: &str) -> String {
        let (status, _, body) = self
            .owner(Method::POST, "/api/v1/agents")
            .json(serde_json::json!({ "label": label }))
            .send(&self.app)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["token"].as_str().unwrap().to_string()
    }
}

pub struct TestRequest {
    builder: axum::http::request::Builder,
    body: Body,
}

impl TestRequest {
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.builder = self.builder.header(name, value);
        self
    }

    pub fn bearer(self, token: &str) -> Self {
        let value = format!("Bearer {token}");
        self.header(header::AUTHORIZATION.as_str(), &value)
    }

    /// Fresh UUID Idempotency-Key, the common case for command requests.
    pub fn idempotency(self) -> Self {
        let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        self.header("idempotency-key", &key)
    }

    pub fn if_match(self, revision: i64) -> Self {
        let value = format!("\"{revision}\"");
        self.header("if-match", &value)
    }

    /// Raw body with whatever Content-Type was already set.
    pub fn body_text(mut self, text: &str) -> Self {
        self.body = Body::from(text.to_string());
        self
    }

    pub fn json(mut self, value: Value) -> Self {
        self.builder = self
            .builder
            .header(header::CONTENT_TYPE, "application/json");
        self.body = Body::from(serde_json::to_vec(&value).unwrap());
        self
    }

    pub async fn send(self, app: &axum::Router) -> (StatusCode, HeaderMap, Value) {
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

pub struct Hierarchy {
    pub project: String,
    pub goal: String,
    pub epic: String,
    pub task: String,
}

/// Owner-created project -> goal -> epic -> one executable task.
pub async fn seed_hierarchy(w: &World) -> Hierarchy {
    let post = |path: String, body: Value| {
        let req = w.owner(Method::POST, &path).idempotency().json(body);
        async move {
            let (status, _, body) = req.send(&w.app).await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            body
        }
    };
    let project = post(
        "/api/v1/projects".into(),
        serde_json::json!({ "name": "P" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let goal = post(
        format!("/api/v1/projects/{project}/goals"),
        serde_json::json!({ "title": "G" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let epic = post(
        format!("/api/v1/projects/{project}/epics"),
        serde_json::json!({ "goal_id": goal, "title": "E" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let task = post(
        format!("/api/v1/projects/{project}/tasks"),
        serde_json::json!({ "epic_id": epic, "title": "T", "type_key": "code" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    Hierarchy {
        project,
        goal,
        epic,
        task,
    }
}

// ── Served-spec validation helpers shared by the contract suites ────────

pub fn load_spec() -> Value {
    let yaml_str = include_str!("../../../../openapi/shepherd.yaml");
    serde_yaml::from_str(yaml_str).expect("failed to parse OpenAPI spec")
}

pub fn resolve_ref<'a>(spec: &'a Value, ref_path: &str) -> &'a Value {
    let path = ref_path.strip_prefix("#/").unwrap_or(ref_path);
    let mut current = spec;
    for segment in path.split('/') {
        current = &current[segment];
    }
    current
}

pub fn resolve_schema(spec: &Value, schema: &Value) -> Value {
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
        Value::Array(arr) => Value::Array(arr.iter().map(|v| resolve_schema(spec, v)).collect()),
        other => other.clone(),
    }
}

pub fn validate_against(spec: &Value, schema_ref: &str, value: &Value) {
    let raw = resolve_ref(spec, schema_ref);
    validate_schema_value(spec, raw, value, schema_ref);
}

pub fn validate_schema_value(spec: &Value, raw_schema: &Value, value: &Value, label: &str) {
    let resolved = resolve_schema(spec, raw_schema);
    let validator = jsonschema_056::Validator::new(&resolved).expect("failed to compile schema");
    if let Err(err) = validator.validate(value) {
        panic!(
            "Schema validation failed for {label}:\n  {err}\nValue: {}",
            serde_json::to_string_pretty(value).unwrap()
        );
    }
}
