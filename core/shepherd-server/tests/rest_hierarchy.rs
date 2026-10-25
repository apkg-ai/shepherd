// Projects, goals and task-types over live HTTP: wiring, header plumbing and
// error mapping. Domain semantics are covered by the core v1_* suites.

mod common;

use axum::http::{Method, StatusCode, header};
use common::{World, world};
use serde_json::json;

async fn create_project(w: &World, name: &str) -> String {
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .json(json!({ "name": name }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn create_goal(w: &World, project: &str, title: &str) -> (String, i64) {
    let (status, _, body) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/goals"))
        .idempotency()
        .json(json!({ "title": title }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["id"].as_str().unwrap().to_string(),
        body["revision"].as_i64().unwrap(),
    )
}

#[tokio::test]
async fn create_project_returns_201_and_committed_state() {
    let w = world().await;
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .json(json!({ "name": "Shepherd", "description": "hub" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "Shepherd");
    assert_eq!(body["revision"], 1);
    assert_eq!(body["settings"]["proposal_gate"], true);
    assert_eq!(body["epic_counts"]["total"], 0);
    assert_eq!(body["archives"], json!([]));

    // The mutation response reflects committed state: a fresh GET agrees.
    let id = body["id"].as_str().unwrap();
    let (status, _, fetched) = w
        .owner(Method::GET, &format!("/api/v1/projects/{id}"))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, body);
}

#[tokio::test]
async fn update_requires_if_match_428_400_412_family() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let path = format!("/api/v1/projects/{project}");

    // Missing If-Match -> 428 before anything else.
    let (status, _, body) = w
        .owner(Method::PATCH, &path)
        .idempotency()
        .json(json!({ "name": "Renamed" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["code"], "precondition_required");

    // Malformed If-Match -> 400.
    let (status, _, body) = w
        .owner(Method::PATCH, &path)
        .idempotency()
        .header("if-match", "1")
        .json(json!({ "name": "Renamed" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "malformed_request");

    // Stale revision -> 412.
    let (status, _, body) = w
        .owner(Method::PATCH, &path)
        .idempotency()
        .if_match(7)
        .json(json!({ "name": "Renamed" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(body["code"], "revision_conflict");

    // Matching revision commits and bumps.
    let (status, _, body) = w
        .owner(Method::PATCH, &path)
        .idempotency()
        .if_match(1)
        .json(json!({ "name": "Renamed" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "Renamed");
    assert_eq!(body["revision"], 2);
}

#[tokio::test]
async fn tampered_cursor_is_422_invalid_cursor() {
    let w = world().await;
    create_project(&w, "P").await;
    let (status, _, body) = w
        .owner(Method::GET, "/api/v1/projects?cursor=bm90LWEtY3Vyc29y")
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "invalid_cursor");
}

#[tokio::test]
async fn get_resource_carries_quoted_revision_etag() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let (goal, _) = create_goal(&w, &project, "G").await;

    let (status, headers, _) = w
        .owner(Method::GET, &format!("/api/v1/projects/{project}"))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::ETAG], "\"1\"");

    let (_, _, updated) = w
        .owner(Method::PATCH, &format!("/api/v1/projects/{project}"))
        .idempotency()
        .if_match(1)
        .json(json!({ "description": "changed" }))
        .send(&w.app)
        .await;
    assert_eq!(updated["revision"], 2);
    let (_, headers, _) = w
        .owner(Method::GET, &format!("/api/v1/projects/{project}"))
        .send(&w.app)
        .await;
    assert_eq!(headers[header::ETAG], "\"2\"");

    // Lists carry no resource ETag; goal reads do.
    let (_, headers, _) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{project}/goals/{goal}"),
        )
        .send(&w.app)
        .await;
    assert_eq!(headers[header::ETAG], "\"1\"");
    let (_, headers, _) = w
        .owner(Method::GET, &format!("/api/v1/projects/{project}/goals"))
        .send(&w.app)
        .await;
    assert!(!headers.contains_key(header::ETAG));
}

#[tokio::test]
async fn lists_exclude_archived_unless_requested() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let (kept, _) = create_goal(&w, &project, "Kept").await;
    let (archived, revision) = create_goal(&w, &project, "Archived").await;

    let (status, _, body) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{project}/goals/{archived}/archive"),
        )
        .idempotency()
        .if_match(revision)
        .json(json!({ "reason": "no longer needed" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["archived"], true);
    assert_eq!(body["archives"][0]["reason"], "no longer needed");

    let ids = |body: &serde_json::Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (_, _, default_list) = w
        .owner(Method::GET, &format!("/api/v1/projects/{project}/goals"))
        .send(&w.app)
        .await;
    assert_eq!(ids(&default_list), vec![kept.clone()]);
    let (_, _, full_list) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{project}/goals?include_archived=true"),
        )
        .send(&w.app)
        .await;
    assert_eq!(ids(&full_list), vec![kept, archived]);
}

#[tokio::test]
async fn task_type_duplicate_key_and_builtin_rows_are_enforced() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let path = format!("/api/v1/projects/{project}/task-types");

    // Builtins are seeded by project creation.
    let (_, _, listed) = w.owner(Method::GET, &path).send(&w.app).await;
    let keys: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"code"), "{keys:?}");

    let (status, _, body) = w
        .owner(Method::POST, &path)
        .idempotency()
        .json(json!({ "key": "review_pass", "label": "Review pass" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["builtin"], false);

    let (status, _, body) = w
        .owner(Method::POST, &path)
        .idempotency()
        .json(json!({ "key": "code", "label": "Duplicate" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn unknown_and_malformed_route_ids_are_404_and_422() {
    let w = world().await;
    let (status, _, body) = w
        .owner(
            Method::GET,
            "/api/v1/projects/00000000-0000-7000-8000-000000000000",
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    let (status, _, body) = w
        .owner(Method::GET, "/api/v1/projects/not-a-uuid")
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation_error");
}

#[tokio::test]
async fn whitespace_only_titles_are_rejected_after_trimming() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let (status, _, body) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/goals"))
        .idempotency()
        .json(json!({ "title": "   " }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation_error");
}

#[tokio::test]
async fn agent_cannot_create_projects_but_can_read_them() {
    let w = world().await;
    let project = create_project(&w, "P").await;
    let agent = w.agent_token("builder").await;

    let (status, _, body) = w
        .req(Method::POST, "/api/v1/projects")
        .bearer(&agent)
        .idempotency()
        .json(json!({ "name": "Nope" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, _, body) = w
        .req(Method::GET, &format!("/api/v1/projects/{project}"))
        .bearer(&agent)
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn oversized_and_wrong_content_type_bodies_are_413_and_415() {
    let w = world().await;

    // Past the 2 MiB generated-validation cap -> 413 payload_too_large.
    let oversized = json!({ "name": "big", "description": "x".repeat(2 * 1024 * 1024) });
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .json(oversized)
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "payload_too_large");

    // A non-JSON content type -> 415 unsupported_media_type.
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .header("content-type", "text/plain")
        .body_text("name=plain")
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
    assert_eq!(body["code"], "unsupported_media_type");

    // Unknown fields are rejected (additionalProperties: false in the contract).
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .json(json!({ "name": "ok", "surprise": true }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation_error");

    // Syntactically broken JSON -> 400 malformed_request.
    let (status, _, body) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .header("content-type", "application/json")
        .body_text("{ not json")
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "malformed_request");
}
