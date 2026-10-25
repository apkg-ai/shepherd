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
