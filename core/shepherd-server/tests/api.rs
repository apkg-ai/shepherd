//! Integration tests: full request → router → response, with contract
//! conformance against `openapi/shepherd.yaml`.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ── Test setup ──────────────────────────────────────────────────────────

async fn test_app() -> axum::Router {
    let (app, _) = test_app_with_store().await;
    app
}

/// Returns both the router and the store, so tests that need to set up
/// state through the domain layer (e.g. claim + session for reject tests)
/// can call store methods directly.
async fn test_app_with_store() -> (axum::Router, shepherd_core::Store) {
    let store = shepherd_core::Store::new_in_memory().await.unwrap();
    let state = shepherd_server::AppState {
        store: store.clone(),
    };
    (shepherd_server::router(state, "does-not-exist"), store)
}

/// Send a request and return `(StatusCode, body as Value)`.
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(path);
    builder = match method {
        "GET" => builder.method("GET"),
        "POST" => builder.method("POST"),
        "PATCH" => builder.method("PATCH"),
        "DELETE" => builder.method("DELETE"),
        _ => panic!("unsupported method: {method}"),
    };

    let req_body = if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&b).unwrap())
    } else {
        Body::empty()
    };

    let req = builder.body(req_body).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// POST helper that returns `(StatusCode, Value)`.
async fn post(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    send(app, "POST", path, Some(body)).await
}

/// GET helper.
async fn get(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    send(app, "GET", path, None).await
}

// ── System tests (existing, adapted) ────────────────────────────────────

#[tokio::test]
async fn health_reports_pass_and_core_version() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

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
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::get("/api/v1/openapi.yaml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/yaml");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"openapi:"));
}

#[tokio::test]
async fn rate_limit_headers_are_present() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert!(response.headers().contains_key("RateLimit-Limit"));
    assert!(response.headers().contains_key("RateLimit-Remaining"));
    assert!(response.headers().contains_key("RateLimit-Reset"));
}

#[tokio::test]
async fn cors_headers_are_present() {
    let app = test_app().await;
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

    // Foreign origins must not be granted cross-origin read access to this
    // unauthenticated local daemon.
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

// ── Project CRUD ────────────────────────────────────────────────────────

#[tokio::test]
async fn project_crud_lifecycle() {
    let app = test_app().await;

    // Create
    let (status, body) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({
            "name": "test-project",
            "description": "A test project",
            "settings": { "review_gate": true }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["name"], "test-project");
    assert_eq!(body["description"], "A test project");
    assert_eq!(body["settings"]["review_gate"], true);
    let project_id = body["id"].as_str().unwrap().to_string();

    // Get
    let (status, body) = get(&app, &format!("/api/v1/projects/{project_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "test-project");

    // List
    let (status, body) = get(&app, "/api/v1/projects").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["has_more"], false);

    // Update
    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{project_id}"),
        Some(serde_json::json!({ "name": "renamed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed");

    // Delete
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{project_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Get after delete → 404
    let (status, body) = get(&app, &format!("/api/v1/projects/{project_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn project_defaults() {
    let app = test_app().await;

    // Create with minimal fields — settings should default.
    let (status, body) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({ "name": "minimal" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["settings"]["review_gate"], true);
    assert_eq!(body["description"], "");
}

// ── Task CRUD ───────────────────────────────────────────────────────────

async fn create_project(app: &axum::Router, name: &str) -> String {
    let (_, body) = post(app, "/api/v1/projects", serde_json::json!({ "name": name })).await;
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn task_crud_lifecycle() {
    let app = test_app().await;
    let pid = create_project(&app, "task-test").await;

    // Create as proposed
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({
            "title": "Write tests",
            "type": "code",
            "status": "proposed"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "proposed");
    assert_eq!(body["title"], "Write tests");
    let tid = body["id"].as_str().unwrap().to_string();

    // Get
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Write tests");

    // Update
    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{pid}/tasks/{tid}"),
        Some(serde_json::json!({ "title": "Write ALL tests" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Write ALL tests");

    // List
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Delete
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/tasks/{tid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Get after delete → 404
    let (status, _) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_create_approved_auto_readies() {
    let app = test_app().await;
    let pid = create_project(&app, "auto-ready-test").await;

    // Create as approved with no dependencies → should auto-ready.
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({
            "title": "Ready task",
            "type": "code",
            "status": "approved"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn task_list_filters() {
    let app = test_app().await;
    let pid = create_project(&app, "filter-test").await;

    // Create a code task and a question task
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "title": "Code task", "type": "code" }),
    )
    .await;
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "title": "Question", "type": "question" }),
    )
    .await;

    // Filter by type
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks?type=code")).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Code task");

    // Filter by status
    let (_, body) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks?status=proposed"),
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
}

// ── Task lifecycle actions ──────────────────────────────────────────────

#[tokio::test]
async fn approve_proposed_task() {
    let app = test_app().await;
    let pid = create_project(&app, "approve-test").await;

    let (_, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "title": "Proposal", "type": "code" }),
    )
    .await;
    let tid = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "proposed");

    // Approve → should become ready (no deps)
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn approve_invalid_status_returns_409() {
    let app = test_app().await;
    let pid = create_project(&app, "bad-approve").await;

    // Create as approved → auto-readies
    let (_, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "title": "Already approved", "type": "code", "status": "approved" }),
    )
    .await;
    let tid = body["id"].as_str().unwrap().to_string();

    // Try to approve a ready task → 409
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:invalid-transition");
}

#[tokio::test]
async fn block_unblock_cancel_lifecycle() {
    let app = test_app().await;
    let pid = create_project(&app, "block-test").await;

    let (_, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "title": "Blockable", "type": "code" }),
    )
    .await;
    let tid = body["id"].as_str().unwrap().to_string();

    // Block
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/block"),
        serde_json::json!({ "reason": "waiting for design" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "blocked");

    // Unblock → restores to proposed
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/unblock"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "proposed");

    // Cancel
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/cancel"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "cancelled");
}

// ── Relations + cycle rejection ─────────────────────────────────────────

async fn create_task(app: &axum::Router, pid: &str, title: &str, status: &str) -> String {
    let (_, body) = post(
        app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({
            "title": title,
            "type": "code",
            "status": status
        }),
    )
    .await;
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn relation_depends_on_crud() {
    let app = test_app().await;
    let pid = create_project(&app, "rel-test").await;
    let a = create_task(&app, &pid, "Task A", "approved").await;
    let b = create_task(&app, &pid, "Task B", "approved").await;

    // Create depends_on: A depends on B
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
        serde_json::json!({
            "type": "depends_on",
            "target_task_id": b
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["type"], "depends_on");
    assert_eq!(body["source_task_id"], a);
    assert_eq!(body["target_task_id"], b);
    let rel_id = body["id"].as_str().unwrap().to_string();

    // List relations for A
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/relations")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Delete the relation
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations/{rel_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify it's gone
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/relations")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn project_relations_bulk_list() {
    let app = test_app().await;
    let pid = create_project(&app, "bulk-rel-test").await;
    let parent = create_task(&app, &pid, "Parent", "approved").await;
    let child = create_task(&app, &pid, "Child", "approved").await;
    let dep = create_task(&app, &pid, "Dep", "approved").await;

    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{parent}/relations"),
        serde_json::json!({ "type": "decomposition", "target_task_id": child }),
    )
    .await;
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{child}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": dep }),
    )
    .await;

    // Bulk list returns both edges in one page.
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/relations")).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(body["has_more"], false);
    assert_eq!(body["next_cursor"], serde_json::Value::Null);
    for item in items {
        assert!(item["id"].is_string());
        assert!(item["source_task_id"].is_string());
        assert!(item["target_task_id"].is_string());
        assert!(item["created_at"].is_string());
    }

    // ?limit=1 pages: walk both pages via next_cursor.
    let (status, page1) = get(&app, &format!("/api/v1/projects/{pid}/relations?limit=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page1["items"].as_array().unwrap().len(), 1);
    assert_eq!(page1["has_more"], true);
    let cursor = page1["next_cursor"].as_str().unwrap();

    let (status, page2) = get(
        &app,
        &format!("/api/v1/projects/{pid}/relations?limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_eq!(page2["has_more"], false);
    assert_ne!(
        page1["items"][0]["id"].as_str().unwrap(),
        page2["items"][0]["id"].as_str().unwrap()
    );

    // Unknown project → 404.
    let (status, body) = get(
        &app,
        "/api/v1/projects/00000000-0000-7000-8000-000000000000/relations",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn relation_cycle_rejected() {
    let app = test_app().await;
    let pid = create_project(&app, "cycle-test").await;
    let a = create_task(&app, &pid, "Task A", "approved").await;
    let b = create_task(&app, &pid, "Task B", "approved").await;
    let c = create_task(&app, &pid, "Task C", "approved").await;

    // A → B → C
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": b }),
    )
    .await;
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{b}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": c }),
    )
    .await;

    // C → A would close a cycle → 409
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{c}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": a }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:dependency-cycle");
}

#[tokio::test]
async fn relation_decomposition_single_parent() {
    let app = test_app().await;
    let pid = create_project(&app, "decomp-test").await;
    let parent1 = create_task(&app, &pid, "Parent 1", "approved").await;
    let parent2 = create_task(&app, &pid, "Parent 2", "approved").await;
    let child = create_task(&app, &pid, "Child", "approved").await;

    // Parent1 → Child (decomposition)
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{parent1}/relations"),
        serde_json::json!({ "type": "decomposition", "target_task_id": child }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Parent2 → Child (second parent) → 409
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{parent2}/relations"),
        serde_json::json!({ "type": "decomposition", "target_task_id": child }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:decomposition-violation");
}

// ── Ready derivation ────────────────────────────────────────────────────

#[tokio::test]
async fn ready_derivation_via_dependency_completion() {
    let app = test_app().await;

    // Use a project with review_gate=false so session success → done directly.
    let (_, proj) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({
            "name": "ready-test",
            "settings": { "review_gate": false }
        }),
    )
    .await;
    let pid = proj["id"].as_str().unwrap();

    // Create A (approved) and B (approved)
    let a = create_task(&app, pid, "Task A", "approved").await;
    let b = create_task(&app, pid, "Task B", "approved").await;

    // Both should be ready (no deps)
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(body["status"], "ready");
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{b}")).await;
    assert_eq!(body["status"], "ready");

    // Add A depends_on B → A should demote to approved
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": b }),
    )
    .await;

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(
        body["status"], "approved",
        "A should be demoted to approved when dep B is not done"
    );

    // Next-task should return B (it's ready, A is not)
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["task"]["id"], b);

    // Claim/session endpoints aren't wired in S4, so we test ready
    // derivation by removing the dependency and seeing A auto-ready.
    let (_, rel_body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/relations")).await;
    let rel_id = rel_body["items"][0]["id"].as_str().unwrap();

    send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations/{rel_id}"),
        None,
    )
    .await;

    // A should now be ready again
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(
        body["status"], "ready",
        "A should auto-ready when dep is removed"
    );
}

#[tokio::test]
async fn next_task_returns_null_when_nothing_ready() {
    let app = test_app().await;
    let pid = create_project(&app, "empty-next").await;

    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["task"].is_null());
}

#[tokio::test]
async fn dependency_demotion_and_re_readying() {
    let app = test_app().await;
    let pid = create_project(&app, "demotion-test").await;

    let a = create_task(&app, &pid, "Task A", "approved").await;
    let b = create_task(&app, &pid, "Task B", "approved").await;

    // Both should be ready
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(body["status"], "ready");

    // Add dep: A depends on B → A demoted to approved
    let (_, rel) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": b }),
    )
    .await;
    let rel_id = rel["id"].as_str().unwrap().to_string();

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(body["status"], "approved");

    // Remove dep → A should auto-ready
    send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations/{rel_id}"),
        None,
    )
    .await;

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(body["status"], "ready");
}

// ── Error responses ─────────────────────────────────────────────────────

#[tokio::test]
async fn not_found_returns_problem_json() {
    let app = test_app().await;
    let fake_uuid = "00000000-0000-0000-0000-000000000000";

    let (status, body) = get(&app, &format!("/api/v1/projects/{fake_uuid}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
    assert_eq!(body["status"], 404);
    assert!(!body["title"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn update_nonexistent_project_returns_404() {
    let app = test_app().await;
    let fake = "00000000-0000-0000-0000-000000000000";

    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{fake}"),
        Some(serde_json::json!({ "name": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn delete_nonexistent_project_returns_404() {
    let app = test_app().await;
    let fake = "00000000-0000-0000-0000-000000000000";

    let (status, body) = send(&app, "DELETE", &format!("/api/v1/projects/{fake}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn update_nonexistent_task_returns_404() {
    let app = test_app().await;
    let pid = create_project(&app, "update-ghost-task").await;
    let fake = "00000000-0000-0000-0000-000000000000";

    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{pid}/tasks/{fake}"),
        Some(serde_json::json!({ "title": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn delete_nonexistent_relation_returns_404() {
    let app = test_app().await;
    let pid = create_project(&app, "del-ghost-rel").await;
    let tid = create_task(&app, &pid, "Has no rels", "approved").await;
    let fake = "00000000-0000-0000-0000-000000000000";

    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/tasks/{tid}/relations/{fake}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["type"], "urn:shepherd:error:not-found");
}

#[tokio::test]
async fn invalid_uuid_returns_error() {
    let app = test_app().await;
    let (status, body) = get(&app, "/api/v1/projects/not-a-uuid").await;
    // Generated validation catches malformed path params before our handler runs.
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "expected 400 or 422, got {status}"
    );
    assert!(body["type"].is_string(), "error should have a type field");
    assert!(
        body["status"].is_number(),
        "error should have a status field"
    );
}

// ── Gap #1: Reject endpoint ─────────────────────────────────────────────

#[tokio::test]
async fn reject_task_in_review_returns_to_ready() {
    let (app, store) = test_app_with_store().await;

    // Create project with review_gate=true (default)
    let pid = create_project(&app, "reject-test").await;
    let pid_id = shepherd_core::ProjectId::from_uuid(pid.parse().unwrap());

    // Create a task and approve it → auto-readies
    let tid = create_task(&app, &pid, "Rejectable", "approved").await;
    let tid_id = shepherd_core::TaskId::from_uuid(tid.parse().unwrap());

    // Drive it to in_review via store: claim → session(succeeded)
    // (claim/session endpoints are S5 — we go through the store directly)
    let now = chrono::Utc::now();
    store
        .claim_task(
            pid_id,
            tid_id,
            &shepherd_core::ClaimRequest {
                identity: shepherd_core::Identity {
                    harness: "test".into(),
                    agent_model: "test".into(),
                    session_id: "s1".into(),
                    label: None,
                },
                ttl_seconds: 600,
            },
            now,
        )
        .await
        .unwrap();

    store
        .create_session(
            pid_id,
            tid_id,
            &shepherd_core::SessionReport {
                identity: shepherd_core::Identity {
                    harness: "test".into(),
                    agent_model: "test".into(),
                    session_id: "s1".into(),
                    label: None,
                },
                started_at: now,
                ended_at: now,
                outcome: shepherd_core::SessionOutcome::Succeeded,
                failure_reason: None,
                summary: Some("done".into()),
                decisions: None,
                knowledge_items: None,
                artifacts: None,
            },
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    // Verify task is now in_review
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(body["status"], "in_review");

    // Reject via REST endpoint
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/reject"),
        serde_json::json!({ "reason": "Tests are failing" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn reject_non_in_review_returns_409() {
    let app = test_app().await;
    let pid = create_project(&app, "bad-reject").await;
    let tid = create_task(&app, &pid, "Proposed task", "proposed").await;

    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/reject"),
        serde_json::json!({ "reason": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:invalid-transition");
}

// ── Gap #2: Review gate effect ──────────────────────────────────────────

#[tokio::test]
async fn review_gate_on_sends_to_in_review() {
    let (app, store) = test_app_with_store().await;

    // review_gate=true (default)
    let pid = create_project(&app, "gate-on").await;
    let pid_id = shepherd_core::ProjectId::from_uuid(pid.parse().unwrap());
    let tid = create_task(&app, &pid, "Gated", "approved").await;
    let tid_id = shepherd_core::TaskId::from_uuid(tid.parse().unwrap());

    let now = chrono::Utc::now();
    store
        .claim_task(
            pid_id,
            tid_id,
            &shepherd_core::ClaimRequest {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                ttl_seconds: 600,
            },
            now,
        )
        .await
        .unwrap();
    store
        .create_session(
            pid_id,
            tid_id,
            &shepherd_core::SessionReport {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                started_at: now,
                ended_at: now,
                outcome: shepherd_core::SessionOutcome::Succeeded,
                failure_reason: None,
                summary: Some("done".into()),
                decisions: None,
                knowledge_items: None,
                artifacts: None,
            },
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(
        body["status"], "in_review",
        "review_gate=true should send succeeded task to in_review"
    );
}

#[tokio::test]
async fn review_gate_off_sends_to_done() {
    let (app, store) = test_app_with_store().await;

    // review_gate=false
    let (_, proj) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({
            "name": "gate-off",
            "settings": { "review_gate": false }
        }),
    )
    .await;
    let pid = proj["id"].as_str().unwrap();
    let pid_id = shepherd_core::ProjectId::from_uuid(pid.parse().unwrap());
    let tid = create_task(&app, pid, "Ungated", "approved").await;
    let tid_id = shepherd_core::TaskId::from_uuid(tid.parse().unwrap());

    let now = chrono::Utc::now();
    store
        .claim_task(
            pid_id,
            tid_id,
            &shepherd_core::ClaimRequest {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                ttl_seconds: 600,
            },
            now,
        )
        .await
        .unwrap();
    store
        .create_session(
            pid_id,
            tid_id,
            &shepherd_core::SessionReport {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                started_at: now,
                ended_at: now,
                outcome: shepherd_core::SessionOutcome::Succeeded,
                failure_reason: None,
                summary: Some("done".into()),
                decisions: None,
                knowledge_items: None,
                artifacts: None,
            },
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(
        body["status"], "done",
        "review_gate=false should send succeeded task directly to done"
    );
}

// ── Gap #3: Pagination ──────────────────────────────────────────────────

#[tokio::test]
async fn project_list_pagination() {
    let app = test_app().await;

    // Create 3 projects; request limit=2
    for i in 0..3 {
        post(
            &app,
            "/api/v1/projects",
            serde_json::json!({ "name": format!("proj-{i}") }),
        )
        .await;
    }

    // Page 1: limit=2
    let (status, body) = get(&app, "/api/v1/projects?limit=2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["has_more"], true);
    let cursor = body["next_cursor"].as_str().unwrap();

    // Page 2: follow cursor
    let (status, body) = get(&app, &format!("/api/v1/projects?limit=2&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["has_more"], false);
}

#[tokio::test]
async fn task_list_pagination() {
    let app = test_app().await;
    let pid = create_project(&app, "task-page-test").await;

    for i in 0..3 {
        post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks"),
            serde_json::json!({ "title": format!("task-{i}"), "type": "code" }),
        )
        .await;
    }

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks?limit=2")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["has_more"], true);
    let cursor = body["next_cursor"].as_str().unwrap();

    let (_, body) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks?limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["has_more"], false);
}

// ── Gap #4: Update project settings ─────────────────────────────────────

#[tokio::test]
async fn update_project_review_gate() {
    let app = test_app().await;

    let (_, body) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({ "name": "settings-test" }),
    )
    .await;
    let pid = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["settings"]["review_gate"], true);

    // Toggle review_gate to false
    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{pid}"),
        Some(serde_json::json!({ "settings": { "review_gate": false } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["review_gate"], false);

    // Verify it persists on GET
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}")).await;
    assert_eq!(body["settings"]["review_gate"], false);
}

// ── Gap #5: Task metadata round-trip ────────────────────────────────────

#[tokio::test]
async fn task_metadata_round_trips() {
    let app = test_app().await;
    let pid = create_project(&app, "meta-test").await;

    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({
            "title": "Metadated",
            "type": "code",
            "metadata": {
                "priority": "high",
                "estimated_hours": 2,
                "tags": ["backend", "urgent"]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["metadata"]["priority"], "high");
    assert_eq!(body["metadata"]["estimated_hours"], 2);
    assert_eq!(body["metadata"]["tags"][0], "backend");

    // Verify on GET
    let tid = body["id"].as_str().unwrap();
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(body["metadata"]["priority"], "high");
    assert_eq!(body["metadata"]["estimated_hours"], 2);
}

// ── Gap #6: Request body validation ─────────────────────────────────────

#[tokio::test]
async fn create_project_missing_name_returns_error() {
    let app = test_app().await;

    // POST with empty body (missing required "name")
    let (status, body) = post(&app, "/api/v1/projects", serde_json::json!({})).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422 for missing required field, got {status}"
    );
    assert!(body["type"].is_string());
}

#[tokio::test]
async fn create_task_missing_required_fields_returns_error() {
    let app = test_app().await;
    let pid = create_project(&app, "validation-test").await;

    // Missing "title" and "type"
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({ "description": "no title or type" }),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {status}"
    );
    assert!(body["type"].is_string());
}

#[tokio::test]
async fn create_relation_invalid_target_uuid_returns_error() {
    let app = test_app().await;
    let pid = create_project(&app, "rel-validation").await;
    let tid = create_task(&app, &pid, "Source", "approved").await;

    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/relations"),
        serde_json::json!({
            "type": "depends_on",
            "target_task_id": "not-a-uuid"
        }),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422 for invalid UUID, got {status}"
    );
    assert!(body["type"].is_string());
}

// ── Ready derivation via store-driven completion ────────────────────────

#[tokio::test]
async fn auto_ready_cascade_when_dependency_completes() {
    let (app, store) = test_app_with_store().await;

    // review_gate=false so session success → done directly
    let (_, proj) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({
            "name": "cascade-test",
            "settings": { "review_gate": false }
        }),
    )
    .await;
    let pid = proj["id"].as_str().unwrap();
    let pid_id = shepherd_core::ProjectId::from_uuid(pid.parse().unwrap());

    let a = create_task(&app, pid, "Task A", "approved").await;
    let b = create_task(&app, pid, "Task B", "approved").await;
    let b_id = shepherd_core::TaskId::from_uuid(b.parse().unwrap());

    // A depends_on B → A demoted
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": b }),
    )
    .await;

    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(body["status"], "approved");

    // Complete B via store (claim + successful session)
    let now = chrono::Utc::now();
    store
        .claim_task(
            pid_id,
            b_id,
            &shepherd_core::ClaimRequest {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                ttl_seconds: 600,
            },
            now,
        )
        .await
        .unwrap();
    store
        .create_session(
            pid_id,
            b_id,
            &shepherd_core::SessionReport {
                identity: shepherd_core::Identity {
                    harness: "t".into(),
                    agent_model: "t".into(),
                    session_id: "s".into(),
                    label: None,
                },
                started_at: now,
                ended_at: now,
                outcome: shepherd_core::SessionOutcome::Succeeded,
                failure_reason: None,
                summary: Some("done".into()),
                decisions: None,
                knowledge_items: None,
                artifacts: None,
            },
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    // B should be done
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{b}")).await;
    assert_eq!(body["status"], "done");

    // A should have auto-readied
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(
        body["status"], "ready",
        "A should auto-ready when its dependency B reaches done"
    );

    // next-task should return A now
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(body["task"]["id"], a);
}

// ── S5: the agent loop over REST ────────────────────────────────────────

/// Identity JSON body fragment for claim/session calls.
fn identity(session_id: &str) -> Value {
    serde_json::json!({
        "harness": "claude-code",
        "agent_model": "opus-5",
        "session_id": session_id
    })
}

/// Claim a task over REST, asserting success. Returns the claim body.
async fn claim(app: &axum::Router, pid: &str, tid: &str, session_id: &str) -> Value {
    let (status, body) = post(
        app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/claim"),
        serde_json::json!({ "identity": identity(session_id), "ttl_seconds": 300 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "claim failed: {body}");
    body
}

/// Report a session over REST. Returns `(status, body)`.
async fn report_session(
    app: &axum::Router,
    pid: &str,
    tid: &str,
    session_id: &str,
    outcome: &str,
    extra: Value,
) -> (StatusCode, Value) {
    let mut body = serde_json::json!({
        "identity": identity(session_id),
        "started_at": "2026-09-10T10:00:00Z",
        "ended_at": "2026-09-10T10:30:00Z",
        "outcome": outcome,
        "summary": "worked on the task"
    });
    if let (Value::Object(base), Value::Object(more)) = (&mut body, extra) {
        base.extend(more);
    }
    post(
        app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/sessions"),
        body,
    )
    .await
}

/// Create a claim whose lease is already expired by backdating `now`
/// (avoids waiting out MIN_TTL). The task ends up `in_progress` with an
/// expired, unswept claim — exactly the crashed-agent state.
async fn claim_expired(store: &shepherd_core::Store, pid: &str, tid: &str, session_id: &str) {
    let pid = shepherd_core::ProjectId::from_uuid(pid.parse().unwrap());
    let tid = shepherd_core::TaskId::from_uuid(tid.parse().unwrap());
    let past = chrono::Utc::now() - chrono::TimeDelta::hours(1);
    store
        .claim_task(
            pid,
            tid,
            &shepherd_core::ClaimRequest {
                identity: shepherd_core::Identity {
                    harness: "claude-code".into(),
                    agent_model: "opus-5".into(),
                    session_id: session_id.into(),
                    label: None,
                },
                ttl_seconds: 30,
            },
            past,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn full_agent_loop_via_rest() {
    let app = test_app().await;

    // Project with the review gate on (default).
    let pid = create_project(&app, "agent-loop").await;
    let a = create_task(&app, &pid, "Design the schema", "approved").await;
    let b = create_task(&app, &pid, "Implement the API", "approved").await;

    // B depends on A → B demoted to approved.
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{b}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": a }),
    )
    .await;

    // 1. next-task offers A (the only ready task).
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["task"]["id"], a);

    // 2. Claim it.
    let claim_body = claim(&app, &pid, &a, "sess-1").await;
    assert_eq!(claim_body["task_id"], a);
    assert!(!claim_body["lease_id"].as_str().unwrap().is_empty());

    // A second claim conflicts.
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/claim"),
        serde_json::json!({ "identity": identity("sess-2"), "ttl_seconds": 300 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:claim-conflict");

    // 3. Context bundle.
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/context")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["task"]["id"], a);
    assert_eq!(body["task"]["status"], "in_progress");

    // Renew the lease mid-work.
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/claim/renew"),
        serde_json::json!({ "identity": identity("sess-1"), "ttl_seconds": 600 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "renew failed: {body}");
    assert_eq!(body["ttl_seconds"], 600);
    assert!(body["renewed_at"].is_string());

    // 4. Report success → in_review (gate on).
    let (status, session) = report_session(
        &app,
        &pid,
        &a,
        "sess-1",
        "succeeded",
        serde_json::json!({
            "decisions": ["normalized the schema"],
            "artifacts": ["https://example.com/pr/1"],
            "knowledge_items": [{
                "type": "decision",
                "title": "Schema shape",
                "content": "Tables are third normal form."
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "session failed: {session}");
    assert_eq!(session["outcome"], "succeeded");
    let sid = session["id"].as_str().unwrap();

    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}")).await;
    assert_eq!(task["status"], "in_review");

    // Session is retrievable and listed.
    let (status, body) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/sessions/{sid}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], sid);
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/sessions")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    // The listed session hydrates its knowledge items too.
    assert_eq!(
        body["items"][0]["knowledge_items"][0]["title"],
        "Schema shape"
    );

    // 5. Human approves the review → A done, B auto-readies.
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{a}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(body["task"]["id"], b, "B should be offered once A is done");
}

#[tokio::test]
async fn failed_session_returns_task_to_ready_with_attempt_history() {
    let app = test_app().await;
    let pid = create_project(&app, "failure-path").await;
    let tid = create_task(&app, &pid, "Flaky work", "approved").await;

    claim(&app, &pid, &tid, "sess-fail").await;
    let (status, session) = report_session(
        &app,
        &pid,
        &tid,
        "sess-fail",
        "failed",
        serde_json::json!({ "failure_reason": "tests would not pass" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(session["outcome"], "failed");
    assert_eq!(session["failure_reason"], "tests would not pass");

    // Task is claimable again with its attempt history kept.
    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(task["status"], "ready");
    assert_eq!(task["attempt_count"], 1);

    // Second attempt succeeds; attempt count keeps growing.
    claim(&app, &pid, &tid, "sess-retry").await;
    let (status, _) =
        report_session(&app, &pid, &tid, "sess-retry", "succeeded", Value::Null).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(task["attempt_count"], 2);

    // Both attempts stay on record; the cursor walks them exactly once,
    // newest first.
    let (_, page1) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/sessions?limit=1"),
    )
    .await;
    assert_eq!(page1["items"].as_array().unwrap().len(), 1);
    assert_eq!(page1["items"][0]["outcome"], "succeeded");
    assert_eq!(page1["has_more"], true);
    let cursor = page1["next_cursor"].as_str().unwrap();
    let (_, page2) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/sessions?limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_eq!(page2["items"][0]["outcome"], "failed");
    assert_eq!(page2["has_more"], false);
}

#[tokio::test]
async fn release_returns_task_to_ready() {
    let app = test_app().await;
    let pid = create_project(&app, "release-path").await;
    let tid = create_task(&app, &pid, "Give up on this", "approved").await;

    claim(&app, &pid, &tid, "sess-quit").await;
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/claim/release"),
        serde_json::json!({ "identity": identity("sess-quit") }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(task["status"], "ready");

    // Renewing after release is 410 Gone.
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/claim/renew"),
        serde_json::json!({ "identity": identity("sess-quit") }),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["type"], "urn:shepherd:error:lease-expired");
}

#[tokio::test]
async fn claim_guard_rejections() {
    let app = test_app().await;
    let pid = create_project(&app, "guards").await;

    // Claiming a non-ready task → 409 task-not-ready.
    let proposed = create_task(&app, &pid, "Not approved yet", "proposed").await;
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{proposed}/claim"),
        serde_json::json!({ "identity": identity("s"), "ttl_seconds": 300 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:task-not-ready");

    // Out-of-range TTL is rejected by validation.
    let ready = create_task(&app, &pid, "Ready task", "approved").await;
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{ready}/claim"),
        serde_json::json!({ "identity": identity("s"), "ttl_seconds": 5 }),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422 for TTL below minimum, got {status}"
    );

    // Renew/release/report by a different identity → 409 claim-conflict.
    claim(&app, &pid, &ready, "sess-owner").await;
    for path in ["claim/renew", "claim/release"] {
        let (status, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{ready}/{path}"),
            serde_json::json!({ "identity": identity("sess-intruder") }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{path} should conflict");
        assert_eq!(body["type"], "urn:shepherd:error:claim-conflict");
    }
    let (status, body) = report_session(
        &app,
        &pid,
        &ready,
        "sess-intruder",
        "succeeded",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:shepherd:error:claim-conflict");

    // Renewing a task that has no claim at all → 410 Gone.
    let unclaimed = create_task(&app, &pid, "Never claimed", "approved").await;
    let (status, body) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{unclaimed}/claim/renew"),
        serde_json::json!({ "identity": identity("s") }),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["type"], "urn:shepherd:error:lease-expired");
}

#[tokio::test]
async fn expired_lease_frees_task_and_blocks_stale_report() {
    let (app, store) = test_app_with_store().await;
    let pid = create_project(&app, "expiry").await;
    let tid = create_task(&app, &pid, "Crashed agent work", "approved").await;

    // Agent claims, then crashes: lease expires unswept.
    claim_expired(&store, &pid, &tid, "sess-crashed").await;
    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(task["status"], "in_progress");

    // The crashed agent's late report is rejected: the lease is gone.
    let (status, body) =
        report_session(&app, &pid, &tid, "sess-crashed", "succeeded", Value::Null).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["type"], "urn:shepherd:error:lease-expired");

    // next-task sweeps the expired lease and offers the task again.
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
    assert_eq!(body["task"]["id"], tid);
    assert_eq!(body["task"]["status"], "ready");

    // Another agent can claim and finish it.
    claim(&app, &pid, &tid, "sess-rescue").await;
    let (status, _) =
        report_session(&app, &pid, &tid, "sess-rescue", "succeeded", Value::Null).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn context_bundle_aggregates_ancestors_knowledge_and_siblings() {
    let app = test_app().await;
    let (_, proj) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({ "name": "context", "settings": { "review_gate": false } }),
    )
    .await;
    let pid = proj["id"].as_str().unwrap().to_string();

    // Ancestor task, completed with a decision and an artifact.
    let ancestor = create_task(&app, &pid, "Pick the storage engine", "approved").await;
    let target = create_task(&app, &pid, "Wire the storage", "approved").await;
    post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{target}/relations"),
        serde_json::json!({ "type": "depends_on", "target_task_id": ancestor }),
    )
    .await;
    claim(&app, &pid, &ancestor, "sess-a").await;
    let (status, _) = report_session(
        &app,
        &pid,
        &ancestor,
        "sess-a",
        "succeeded",
        serde_json::json!({
            "decisions": ["SQLite with WAL"],
            "artifacts": ["https://example.com/pr/7"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Project-level knowledge.
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge"),
        serde_json::json!({
            "type": "note",
            "title": "Conventions",
            "content": "Cursor pagination everywhere.",
            "scope": "project"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A decomposition parent: its summaries belong in the bundle too.
    let parent = create_task(&app, &pid, "Storage epic", "approved").await;
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{parent}/relations"),
        serde_json::json!({ "type": "decomposition", "target_task_id": target }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // An in-flight sibling claimed by someone else.
    let sibling = create_task(&app, &pid, "Parallel work", "approved").await;
    claim(&app, &pid, &sibling, "sess-sibling").await;

    // Claim the target and pull its bundle.
    claim(&app, &pid, &target, "sess-b").await;
    let (status, bundle) = get(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{target}/context"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bundle["task"]["id"], target);

    // Both the dependency ancestor and the decomposition parent appear.
    let ancestors = bundle["ancestor_summaries"].as_array().unwrap();
    assert_eq!(ancestors.len(), 2);
    let ancestor_ids: Vec<&str> = ancestors
        .iter()
        .map(|a| a["task_id"].as_str().unwrap())
        .collect();
    assert!(ancestor_ids.contains(&ancestor.as_str()));
    assert!(ancestor_ids.contains(&parent.as_str()));
    let dep_ancestor = ancestors
        .iter()
        .find(|a| a["task_id"] == ancestor.as_str())
        .unwrap();
    assert_eq!(dep_ancestor["decisions"][0], "SQLite with WAL");

    assert_eq!(bundle["artifacts"][0], "https://example.com/pr/7");
    assert_eq!(bundle["project_knowledge"][0]["title"], "Conventions");

    let siblings = bundle["sibling_tasks"].as_array().unwrap();
    assert_eq!(siblings.len(), 1, "only the sibling, not the target itself");
    assert_eq!(siblings[0]["task_id"], sibling);
    assert_eq!(siblings[0]["claimed_by"]["session_id"], "sess-sibling");
}

#[tokio::test]
async fn background_sweeper_frees_expired_lease_without_traffic() {
    let (app, store) = test_app_with_store().await;
    let pid = create_project(&app, "sweeper").await;
    let tid = create_task(&app, &pid, "Crashed and forgotten", "approved").await;

    // A crashed agent's lease, already expired.
    claim_expired(&store, &pid, &tid, "sess-crashed").await;
    let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
    assert_eq!(task["status"], "in_progress");

    // The background sweeper alone must free the task — no next-task call.
    let sweeper = shepherd_server::spawn_claim_sweeper(store, std::time::Duration::from_millis(20));
    let mut freed = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (_, task) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}")).await;
        if task["status"] == "ready" {
            freed = true;
            break;
        }
    }
    sweeper.abort();
    assert!(
        freed,
        "sweeper should return the expired-lease task to ready"
    );
}

// ── S5: knowledge CRUD ──────────────────────────────────────────────────

#[tokio::test]
async fn knowledge_crud_and_scope_filters() {
    let app = test_app().await;
    let pid = create_project(&app, "knowledge").await;
    let tid = create_task(&app, &pid, "Produces knowledge", "approved").await;

    // Project-scoped item.
    let (status, project_item) = post(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge"),
        serde_json::json!({
            "type": "decision",
            "title": "Error model",
            "content": "RFC 9457 everywhere.",
            "scope": "project"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(project_item["scope"], "project");
    let kid = project_item["id"].as_str().unwrap().to_string();

    // Task-scoped item.
    let (status, task_item) = post(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge"),
        serde_json::json!({
            "type": "link",
            "title": "The PR",
            "content": "https://example.com/pr/2",
            "scope": "task",
            "task_id": tid
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(task_item["task_id"], tid);

    // List all, then filter by scope, type, and task.
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/knowledge")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    let (_, body) = get(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge?scope=project"),
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["id"], kid.as_str());
    let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/knowledge?type=link")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (_, body) = get(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge?task_id={tid}"),
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Cursor pagination walks all items exactly once.
    let (_, page1) = get(&app, &format!("/api/v1/projects/{pid}/knowledge?limit=1")).await;
    assert_eq!(page1["items"].as_array().unwrap().len(), 1);
    assert_eq!(page1["has_more"], true);
    let cursor = page1["next_cursor"].as_str().unwrap();
    let (_, page2) = get(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge?limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_eq!(page2["has_more"], false);
    assert_ne!(page1["items"][0]["id"], page2["items"][0]["id"]);

    // Get, delete, gone.
    let (status, body) = get(&app, &format!("/api/v1/projects/{pid}/knowledge/{kid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Error model");
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{pid}/knowledge/{kid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = get(&app, &format!("/api/v1/projects/{pid}/knowledge/{kid}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── S5: export / import ─────────────────────────────────────────────────

#[tokio::test]
async fn export_import_roundtrip_with_normalization() {
    let app = test_app().await;
    let (_, proj) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({ "name": "exported", "settings": { "review_gate": false } }),
    )
    .await;
    let pid = proj["id"].as_str().unwrap().to_string();

    // One done task whose session produced knowledge, one mid-work task.
    let done = create_task(&app, &pid, "Finished work", "approved").await;
    claim(&app, &pid, &done, "sess-1").await;
    report_session(
        &app,
        &pid,
        &done,
        "sess-1",
        "succeeded",
        serde_json::json!({
            "knowledge_items": [{
                "type": "decision",
                "title": "Produced in session",
                "content": "Travels through export and import."
            }]
        }),
    )
    .await;
    let midwork = create_task(&app, &pid, "Mid-flight work", "approved").await;
    claim(&app, &pid, &midwork, "sess-2").await;
    post(
        &app,
        &format!("/api/v1/projects/{pid}/knowledge"),
        serde_json::json!({
            "type": "note",
            "title": "Glossary",
            "content": "Everything is a task.",
            "scope": "project"
        }),
    )
    .await;

    // Export.
    let (status, doc) = get(&app, &format!("/api/v1/projects/{pid}/export")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["version"], "1.0.0");
    assert_eq!(doc["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(doc["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(doc["knowledge"].as_array().unwrap().len(), 2);

    // Import → a new project with matching counts.
    let (status, result) = post(&app, "/api/v1/projects/import", doc.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "import failed: {result}");
    let new_pid = result["project_id"].as_str().unwrap();
    assert_ne!(new_pid, pid, "import must create a new project");
    assert_eq!(result["task_count"], 2);
    assert_eq!(result["session_count"], 1);
    assert_eq!(result["knowledge_count"], 2);

    // Claims are not exported: the in_progress task normalized to ready.
    let (_, tasks) = get(&app, &format!("/api/v1/projects/{new_pid}/tasks")).await;
    let imported_midwork = tasks["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Mid-flight work")
        .unwrap();
    assert_eq!(imported_midwork["status"], "ready");

    // Knowledge references are remapped to the NEW task and session ids:
    // querying by the imported done task's id finds the session knowledge.
    let imported_done = tasks["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Finished work")
        .unwrap();
    let new_done_id = imported_done["id"].as_str().unwrap();
    assert_ne!(new_done_id, done, "imported task must get a fresh id");
    let (_, ki) = get(
        &app,
        &format!("/api/v1/projects/{new_pid}/knowledge?task_id={new_done_id}"),
    )
    .await;
    assert_eq!(ki["items"].as_array().unwrap().len(), 1);
    assert_eq!(ki["items"][0]["title"], "Produced in session");
    let (_, sessions) = get(
        &app,
        &format!("/api/v1/projects/{new_pid}/tasks/{new_done_id}/sessions"),
    )
    .await;
    assert_eq!(
        ki["items"][0]["session_id"], sessions["items"][0]["id"],
        "knowledge must point at the imported session, not the old one"
    );

    // Incompatible schema version → 422.
    let mut bad = doc;
    bad["version"] = Value::String("2.0.0".into());
    let (status, body) = post(&app, "/api/v1/projects/import", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["type"], "urn:shepherd:error:import-schema-mismatch");
}

// ── S5: import-then-delete must not 404 (issue #41) ────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_then_delete_does_not_404() {
    // File-backed store: the 4-connection pool reproduces the WAL visibility
    // race that the in-memory single-connection pool would hide.
    let dir = tempfile::tempdir().unwrap();
    let store = shepherd_core::Store::open(dir.path().join("import-del.db"))
        .await
        .unwrap();
    let state = shepherd_server::AppState { store };
    let app = shepherd_server::router(state, "does-not-exist");

    // Seed a pre-existing project (unrelated to the import).
    let pre_pid = create_project(&app, "pre-existing").await;

    // Build a minimal exportable project with a task, export it.
    let src_pid = create_project(&app, "to-export").await;
    let _tid = create_task(&app, &src_pid, "importable task", "approved").await;

    let (_, doc) = get(&app, &format!("/api/v1/projects/{src_pid}/export")).await;
    let (status, result) = post(&app, "/api/v1/projects/import", doc).await;
    assert_eq!(status, StatusCode::CREATED, "import must succeed: {result}");
    let imported_pid = result["project_id"].as_str().unwrap().to_string();

    // Immediately delete the IMPORTED project → must be 204, not 404.
    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/api/v1/projects/{imported_pid}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "delete of just-imported project must not 404: {body}"
    );

    // Immediately delete the UNRELATED pre-existing project → must also be 204.
    let (status, body) = send(&app, "DELETE", &format!("/api/v1/projects/{pre_pid}"), None).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "delete of unrelated project after import must not 404: {body}"
    );
}

// ── S5: concurrency — N parallel claims, exactly one winner ─────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn n_parallel_claims_exactly_one_winner() {
    // A file-backed store: the in-memory pool has a single connection which
    // serializes writes and would hide the race this test exists to catch.
    let dir = tempfile::tempdir().unwrap();
    let store = shepherd_core::Store::open(dir.path().join("claims.db"))
        .await
        .unwrap();
    let state = shepherd_server::AppState { store };
    let app = shepherd_server::router(state, "does-not-exist");

    let pid = create_project(&app, "contested").await;
    let tid = create_task(&app, &pid, "One winner only", "approved").await;

    const N: usize = 16;
    let mut handles = Vec::new();
    for i in 0..N {
        let app = app.clone();
        let path = format!("/api/v1/projects/{pid}/tasks/{tid}/claim");
        handles.push(tokio::spawn(async move {
            let body = serde_json::json!({
                "identity": identity(&format!("sess-{i}")),
                "ttl_seconds": 300
            });
            let (status, _) = post(&app, &path, body).await;
            status
        }));
    }

    let mut winners = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.await.unwrap() {
            StatusCode::CREATED => winners += 1,
            StatusCode::CONFLICT => conflicts += 1,
            other => panic!("unexpected status under contention: {other}"),
        }
    }
    assert_eq!(winners, 1, "exactly one claim must win");
    assert_eq!(conflicts, N - 1, "all others must conflict");
}

// ── Contract conformance (schema validation) ────────────────────────────

mod contract {
    use super::*;
    use jsonschema_055::Validator;

    /// Load the OpenAPI spec and extract the response schema for a given
    /// operation. Returns the resolved JSON schema suitable for validation.
    fn load_spec() -> Value {
        let yaml_str = include_str!("../../../openapi/shepherd.yaml");
        serde_yaml::from_str(yaml_str).expect("failed to parse OpenAPI spec")
    }

    /// Resolve a JSON `$ref` within the spec.
    fn resolve_ref<'a>(spec: &'a Value, ref_path: &str) -> &'a Value {
        let path = ref_path.strip_prefix("#/").unwrap_or(ref_path);
        let mut current = spec;
        for segment in path.split('/') {
            current = &current[segment];
        }
        current
    }

    /// Recursively resolve all `$ref` in a schema, returning a self-contained
    /// JSON schema. This is a simple resolver for our spec's structure.
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

    /// Build a JSON Schema validator from the spec's schema for a given
    /// path (e.g. `"components/schemas/Project"`).
    fn schema_validator(spec: &Value, schema_ref: &str) -> Validator {
        let raw = resolve_ref(spec, schema_ref);
        let resolved = resolve_schema(spec, raw);
        Validator::new(&resolved).expect("failed to compile schema")
    }

    /// Validate a value against a spec schema.
    fn validate(spec: &Value, schema_ref: &str, value: &Value) {
        let validator = schema_validator(spec, schema_ref);
        if let Err(err) = validator.validate(value) {
            panic!(
                "Schema validation failed for {schema_ref}:\n  {err}\nValue: {}",
                serde_json::to_string_pretty(value).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn project_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        let (_, body) = post(
            &app,
            "/api/v1/projects",
            serde_json::json!({
                "name": "contract-test",
                "description": "test desc"
            }),
        )
        .await;

        validate(&spec, "components/schemas/Project", &body);
    }

    #[tokio::test]
    async fn project_list_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        post(
            &app,
            "/api/v1/projects",
            serde_json::json!({ "name": "list-contract" }),
        )
        .await;

        let (_, body) = get(&app, "/api/v1/projects").await;
        validate(&spec, "components/schemas/ProjectList", &body);
    }

    #[tokio::test]
    async fn task_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "task-contract").await;

        let (_, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks"),
            serde_json::json!({
                "title": "Contract task",
                "type": "code",
                "status": "approved"
            }),
        )
        .await;

        validate(&spec, "components/schemas/Task", &body);
    }

    #[tokio::test]
    async fn task_list_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "tasklist-contract").await;
        create_task(&app, &pid, "Contract task", "proposed").await;

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks")).await;
        validate(&spec, "components/schemas/TaskList", &body);
    }

    #[tokio::test]
    async fn relation_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "rel-contract").await;
        let a = create_task(&app, &pid, "A", "approved").await;
        let b = create_task(&app, &pid, "B", "approved").await;

        let (_, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
            serde_json::json!({ "type": "depends_on", "target_task_id": b }),
        )
        .await;

        validate(&spec, "components/schemas/Relation", &body);
    }

    #[tokio::test]
    async fn relation_list_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "rellist-contract").await;
        let a = create_task(&app, &pid, "A", "approved").await;
        let b = create_task(&app, &pid, "B", "approved").await;
        post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{a}/relations"),
            serde_json::json!({ "type": "depends_on", "target_task_id": b }),
        )
        .await;

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{a}/relations")).await;
        validate(&spec, "components/schemas/RelationList", &body);
    }

    #[tokio::test]
    async fn next_task_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "next-contract").await;
        create_task(&app, &pid, "Ready one", "approved").await;

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
        validate(&spec, "components/schemas/NextTaskResult", &body);
    }

    #[tokio::test]
    async fn next_task_null_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "next-null-contract").await;

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/next-task")).await;
        validate(&spec, "components/schemas/NextTaskResult", &body);
    }

    #[tokio::test]
    async fn error_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let fake = "00000000-0000-0000-0000-000000000000";

        let (_, body) = get(&app, &format!("/api/v1/projects/{fake}")).await;
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    #[tokio::test]
    async fn conflict_error_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "conflict-contract").await;
        let tid = create_task(&app, &pid, "Ready task", "approved").await;

        // Approving an already-ready task is an invalid transition → 409.
        let (status, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{tid}/approve"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["type"], "urn:shepherd:error:invalid-transition");
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    #[tokio::test]
    async fn request_validation_error_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        // Missing required "name" — rejected by the generated validation
        // layer, remapped to the shepherd ProblemDetail shape.
        let (status, body) = post(&app, "/api/v1/projects", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["type"], "urn:shepherd:error:validation-error");
        assert_eq!(body["errors"][0]["field"], "/body/name");
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    #[tokio::test]
    async fn invalid_path_parameter_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        let (status, body) = get(&app, "/api/v1/projects/not-a-uuid").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["type"], "urn:shepherd:error:validation-error");
        assert_eq!(body["errors"][0]["field"], "/path/project_id");
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    #[tokio::test]
    async fn malformed_json_body_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/projects")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{ not json"))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["type"], "urn:shepherd:error:validation-error");
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    #[tokio::test]
    async fn unsupported_media_type_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/projects")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("{\"name\":\"x\"}"))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        // The spec documents only 401/404/409/422/429/500; the remap layer
        // folds pre-domain rejections into the documented 422.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["type"], "urn:shepherd:error:validation-error");
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }

    // ── S5: agent-loop schemas ──────────────────────────────────────────

    /// Set up a project with one claimed task; returns `(pid, tid)`.
    async fn claimed_task(app: &axum::Router, name: &str) -> (String, String) {
        let pid = create_project(app, name).await;
        let tid = create_task(app, &pid, "Claimed task", "approved").await;
        claim(app, &pid, &tid, "sess-contract").await;
        (pid, tid)
    }

    #[tokio::test]
    async fn claim_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "claim-contract").await;
        let tid = create_task(&app, &pid, "Claim me", "approved").await;

        let (_, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{tid}/claim"),
            serde_json::json!({ "identity": identity("sess-c"), "ttl_seconds": 300 }),
        )
        .await;
        validate(&spec, "components/schemas/Claim", &body);

        // Renewed claim conforms too (renewed_at now set).
        let (_, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{tid}/claim/renew"),
            serde_json::json!({ "identity": identity("sess-c") }),
        )
        .await;
        validate(&spec, "components/schemas/Claim", &body);
    }

    #[tokio::test]
    async fn context_bundle_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let (pid, tid) = claimed_task(&app, "context-contract").await;

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/tasks/{tid}/context")).await;
        validate(&spec, "components/schemas/ContextBundle", &body);
    }

    #[tokio::test]
    async fn session_responses_conform_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let (pid, tid) = claimed_task(&app, "session-contract").await;

        let (_, body) = report_session(
            &app,
            &pid,
            &tid,
            "sess-contract",
            "succeeded",
            serde_json::json!({
                "decisions": ["a decision"],
                "artifacts": ["https://example.com/pr/9"],
                "knowledge_items": [{
                    "type": "note",
                    "title": "Learned",
                    "content": "Something reusable."
                }]
            }),
        )
        .await;
        validate(&spec, "components/schemas/Session", &body);

        let (_, body) = get(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{tid}/sessions"),
        )
        .await;
        validate(&spec, "components/schemas/SessionList", &body);
    }

    #[tokio::test]
    async fn knowledge_responses_conform_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "knowledge-contract").await;

        let (_, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/knowledge"),
            serde_json::json!({
                "type": "decision",
                "title": "Contract knowledge",
                "content": "Validated against the spec.",
                "scope": "project"
            }),
        )
        .await;
        validate(&spec, "components/schemas/KnowledgeItem", &body);

        let (_, body) = get(&app, &format!("/api/v1/projects/{pid}/knowledge")).await;
        validate(&spec, "components/schemas/KnowledgeItemList", &body);
    }

    #[tokio::test]
    async fn export_and_import_conform_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let (pid, tid) = claimed_task(&app, "export-contract").await;
        report_session(&app, &pid, &tid, "sess-contract", "succeeded", Value::Null).await;

        let (_, doc) = get(&app, &format!("/api/v1/projects/{pid}/export")).await;
        validate(&spec, "components/schemas/ExportDocument", &doc);

        let (_, result) = post(&app, "/api/v1/projects/import", doc).await;
        validate(&spec, "components/schemas/ImportResult", &result);
    }

    #[tokio::test]
    async fn gone_error_response_conforms_to_spec() {
        let spec = load_spec();
        let app = test_app().await;
        let pid = create_project(&app, "gone-contract").await;
        let tid = create_task(&app, &pid, "Never claimed", "approved").await;

        let (status, body) = post(
            &app,
            &format!("/api/v1/projects/{pid}/tasks/{tid}/claim/renew"),
            serde_json::json!({ "identity": identity("sess-x") }),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        validate(&spec, "components/schemas/ProblemDetail", &body);
    }
}

// ── SSE & event tests ─────────────────────────────────────────────────

#[tokio::test]
async fn sse_endpoint_returns_event_stream_content_type() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::get("/api/v1/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type header")
        .to_str()
        .unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "expected text/event-stream, got: {ct}"
    );
}

#[tokio::test]
async fn sse_endpoint_with_project_filter() {
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::get("/api/v1/events?project_id=019421a5-7e6e-7000-8000-000000000001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn rest_mutation_emits_domain_event() {
    let (app, store) = test_app_with_store().await;
    let mut rx = store.subscribe();

    // POST a project through the REST API.
    let (status, body) = post(
        &app,
        "/api/v1/projects",
        serde_json::json!({ "name": "event-test" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let project_id = body["id"].as_str().unwrap();

    // The store's event bus should have received a project.created event.
    let event = rx.try_recv().expect("expected a domain event");
    assert_eq!(event.event_type(), "project.created");
    assert_eq!(event.project_id().to_string(), project_id);
}

#[tokio::test]
async fn rest_create_task_emits_created_and_status_changed() {
    let (app, store) = test_app_with_store().await;
    let pid = create_project(&app, "evt-task-test").await;
    let mut rx = store.subscribe();

    // Create a task as approved (triggers auto-ready).
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks"),
        serde_json::json!({
            "title": "event task",
            "type": "code",
            "status": "approved",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Should emit task.created + task.status_changed (approved → ready).
    let mut events = vec![];
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let types: Vec<&str> = events.iter().map(|e| e.event_type()).collect();
    assert!(
        types.contains(&"task.created"),
        "expected task.created in {types:?}"
    );
    assert!(
        types.contains(&"task.status_changed"),
        "expected task.status_changed in {types:?}"
    );
}

#[tokio::test]
async fn rest_full_agent_loop_emits_all_events() {
    let (app, store) = test_app_with_store().await;
    let pid = create_project(&app, "evt-loop-test").await;
    let tid = create_task(&app, &pid, "loop-task", "approved").await;
    let mut rx = store.subscribe();

    // Claim.
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/claim"),
        serde_json::json!({
            "identity": identity("loop-sess"),
            "ttl_seconds": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Report session.
    let (status, _) = post(
        &app,
        &format!("/api/v1/projects/{pid}/tasks/{tid}/sessions"),
        serde_json::json!({
            "identity": identity("loop-sess"),
            "started_at": "2026-01-01T00:00:00Z",
            "ended_at": "2026-01-01T01:00:00Z",
            "outcome": "succeeded",
            "summary": "done",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Collect all events.
    let mut events = vec![];
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let types: Vec<&str> = events.iter().map(|e| e.event_type()).collect();

    // Expected events from claim + session report:
    assert!(
        types.contains(&"claim.acquired"),
        "missing claim.acquired in {types:?}"
    );
    assert!(
        types.contains(&"task.status_changed"),
        "missing task.status_changed in {types:?}"
    );
    assert!(
        types.contains(&"claim.released"),
        "missing claim.released in {types:?}"
    );
    assert!(
        types.contains(&"session.recorded"),
        "missing session.recorded in {types:?}"
    );
}

#[tokio::test]
async fn sse_delivers_event_frames_end_to_end() {
    // Gap #1: Read actual SSE frames from the response body.
    let (app, store) = test_app_with_store().await;

    // Connect to the SSE endpoint.
    let response = app
        .oneshot(Request::get("/api/v1/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The subscription is now active. Emit an event through the store.
    store
        .create_project(&shepherd_core::ProjectCreate {
            name: "sse-frame-test".into(),
            description: None,
            settings: None,
        })
        .await
        .unwrap();

    // Read from the streaming body — the event should appear as SSE frames.
    let mut body = response.into_body();
    let frame = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        http_body_util::BodyExt::frame(&mut body),
    )
    .await
    .expect("timed out waiting for SSE frame")
    .expect("body stream ended")
    .expect("body error");

    let bytes = frame.into_data().expect("expected a data frame");
    let text = String::from_utf8(bytes.to_vec()).unwrap();

    // SSE format: "event: project.created\ndata: {...}\n\n"
    assert!(
        text.contains("event: project.created"),
        "SSE frame missing event type: {text}"
    );
    assert!(
        text.contains("\"project_id\""),
        "SSE frame missing project_id in data: {text}"
    );
    assert!(
        text.contains("\"name\""),
        "SSE frame missing name in data: {text}"
    );
    assert!(
        text.contains("sse-frame-test"),
        "SSE frame missing project name value: {text}"
    );
}

#[tokio::test]
async fn sse_project_filter_excludes_other_projects() {
    // Gap #2: Verify project_id filter actually filters events.
    let (app, store) = test_app_with_store().await;

    // Create two projects via the store.
    let p_wanted = store
        .create_project(&shepherd_core::ProjectCreate {
            name: "wanted".into(),
            description: None,
            settings: None,
        })
        .await
        .unwrap();
    let _p_other = store
        .create_project(&shepherd_core::ProjectCreate {
            name: "other".into(),
            description: None,
            settings: None,
        })
        .await
        .unwrap();

    // Connect with project filter for the wanted project.
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/events?project_id={}", p_wanted.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Emit an event for the OTHER project — should be filtered out.
    store
        .update_project(
            _p_other.id,
            &shepherd_core::ProjectUpdate {
                name: Some("renamed-other".into()),
                description: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Emit an event for the WANTED project — should pass the filter.
    store
        .create_task(
            p_wanted.id,
            &shepherd_core::TaskCreate {
                title: "wanted-task".into(),
                description: None,
                task_type: shepherd_core::TaskType::Code,
                status: Some(shepherd_core::TaskStatus::Proposed),
                metadata: None,
                assignee: None,
                graph_role: None,
            },
        )
        .await
        .unwrap();

    // Read from the body — first real frame should be for the wanted project.
    let mut body = response.into_body();
    let frame = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        http_body_util::BodyExt::frame(&mut body),
    )
    .await
    .expect("timed out waiting for SSE frame")
    .expect("body stream ended")
    .expect("body error");

    let bytes = frame.into_data().expect("expected a data frame");
    let text = String::from_utf8(bytes.to_vec()).unwrap();

    // The frame should be for the wanted project's task, not the other project's update.
    assert!(
        text.contains("task.created"),
        "expected task.created from wanted project, got: {text}"
    );
    assert!(
        text.contains("wanted-task"),
        "expected wanted-task title, got: {text}"
    );
    assert!(
        !text.contains("renamed-other"),
        "other project's event should have been filtered out: {text}"
    );
}

#[tokio::test]
async fn sse_malformed_project_filter_returns_422() {
    // The spec declares a UUID pattern + 422 for the filter — a malformed
    // id must be rejected, not silently stream nothing. (An uppercase UUID
    // parses to the same value via the uuid crate and filters correctly.)
    let app = test_app().await;

    for bad in ["not-a-uuid", "019421a5-7e6e-7000-8000-00000000000"] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/events?project_id={bad}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "expected 422 for filter {bad:?}"
        );
        let ct = response.headers()[header::CONTENT_TYPE].to_str().unwrap();
        assert!(
            ct.contains("application/problem+json"),
            "expected problem+json, got {ct}"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["type"], "urn:shepherd:error:validation-error");
    }
}

#[tokio::test]
async fn sse_lagged_client_is_disconnected() {
    // A client that falls behind the bus buffer must be disconnected (it
    // then reconnects and refetches) — never silently skip lost events.
    let (app, store) = test_app_with_store().await;

    // Connect; the handler's receiver is subscribed but not yet polled.
    let response = app
        .oneshot(Request::get("/api/v1/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Overflow the 256-slot broadcast buffer while nobody is polling.
    for i in 0..300 {
        store
            .create_project(&shepherd_core::ProjectCreate {
                name: format!("flood-{i}"),
                description: None,
                settings: None,
            })
            .await
            .unwrap();
    }

    // The first poll yields Lagged → the stream must END (None), not hang
    // and not silently resume.
    let mut body = response.into_body();
    let frame = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        http_body_util::BodyExt::frame(&mut body),
    )
    .await
    .expect("timed out — lagged client was not disconnected");
    assert!(
        frame.is_none(),
        "expected the stream to end after lag, got a frame: {frame:?}"
    );
}
