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
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert!(
        response
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
}
