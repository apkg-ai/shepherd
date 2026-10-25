// Review-policy, eligibility and bound negatives over live HTTP, plus the
// ETag/replay proofs for the content resources — completing the plan/07
// error-code matrix at the public boundary.

mod common;

use axum::http::{Method, StatusCode, header};
use common::{World, seed_hierarchy, world};
use serde_json::{Value, json};

async fn get(w: &World, path: &str) -> (StatusCode, Value) {
    let (status, _, body) = w.owner(Method::GET, path).send(&w.app).await;
    (status, body)
}

/// Agent claims the task, files a finding, and reports success, producing a
/// pending submission under the project's work-review policy.
async fn agent_submission(w: &World, project: &str, task: &str, agent: &str) -> Value {
    let (_, current) = get(w, &format!("/api/v1/projects/{project}/tasks/{task}")).await;
    let (status, _, grant) = w
        .req(
            Method::POST,
            &format!("/api/v1/projects/{project}/tasks/{task}/claims"),
        )
        .bearer(agent)
        .idempotency()
        .if_match(current["revision"].as_i64().unwrap())
        .json(json!({ "phase": "execute" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let (status, _, document) = w
        .req(
            Method::POST,
            &format!("/api/v1/projects/{project}/documents"),
        )
        .bearer(agent)
        .idempotency()
        .json(json!({
            "owner_kind": "task",
            "owner_id": task,
            "kind": "finding",
            "title": "Findings",
            "body": "what was learned",
            "links": [],
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{document}");
    let claim_id = grant["claim"]["id"].as_str().unwrap();
    let (status, _, session) = w
        .req(
            Method::POST,
            &format!("/api/v1/projects/{project}/claims/{claim_id}/report"),
        )
        .bearer(agent)
        .idempotency()
        .header("x-lease-token", grant["lease_token"].as_str().unwrap())
        .json(json!({
            "outcome": "succeeded",
            "summary": "done",
            "document_revision_ids": [document["latest_revision_id"]],
            "links": [],
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let (_, pending) = get(
        w,
        &format!("/api/v1/projects/{project}/submissions?status=pending"),
    )
    .await;
    pending["items"][0].clone()
}

#[tokio::test]
async fn claiming_an_unaccepted_proposal_is_409_not_eligible() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let agent = w.agent_token("proposer").await;
    // Agent-created tasks start as proposals under the default gate.
    let (status, _, proposed) = w
        .req(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks", h.project),
        )
        .bearer(&agent)
        .idempotency()
        .json(json!({ "epic_id": h.epic, "title": "Proposed", "type_key": "code" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{proposed}");
    assert_eq!(proposed["status"], "proposed");

    let (status, _, body) = w
        .owner(
            Method::POST,
            &format!(
                "/api/v1/projects/{}/tasks/{}/claims",
                h.project,
                proposed["id"].as_str().unwrap()
            ),
        )
        .idempotency()
        .if_match(proposed["revision"].as_i64().unwrap())
        .json(json!({ "phase": "execute" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "not_eligible");
}

#[tokio::test]
async fn agent_policy_review_negatives_map_to_403_409_and_422() {
    let w = world().await;
    // Agent work-review policy: humans lack the capability, producers cannot
    // hold the review claim, settled submissions cannot be re-claimed.
    let (status, _, project) = w
        .owner(Method::POST, "/api/v1/projects")
        .idempotency()
        .json(json!({
            "name": "Agent reviewed",
            "settings": {
                "proposal_gate": false,
                "planning_required": false,
                "plan_review": "human",
                "work_review": "agent",
            },
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project = project["id"].as_str().unwrap().to_string();
    let (_, _, goal) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/goals"))
        .idempotency()
        .json(json!({ "title": "G" }))
        .send(&w.app)
        .await;
    let (_, _, epic) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/epics"))
        .idempotency()
        .json(json!({ "goal_id": goal["id"], "title": "E" }))
        .send(&w.app)
        .await;
    let (_, _, task) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/tasks"))
        .idempotency()
        .json(json!({ "epic_id": epic["id"], "title": "T", "type_key": "code" }))
        .send(&w.app)
        .await;
    let task = task["id"].as_str().unwrap().to_string();
    let producer = w.agent_token("producer").await;
    let reviewer = w.agent_token("reviewer").await;
    let submission = agent_submission(&w, &project, &task, &producer).await;
    let submission_id = submission["id"].as_str().unwrap();
    let revision = submission["revision"].as_i64().unwrap();
    let reviews = format!("/api/v1/projects/{project}/submissions/{submission_id}/reviews");

    // A human reviewing under the agent policy lacks the capability: 403.
    let (status, _, body) = w
        .owner(Method::POST, &reviews)
        .idempotency()
        .if_match(revision)
        .json(json!({ "decision": "approve", "reason": "lgtm" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "forbidden");

    // The producer cannot hold the review claim for its own submission.
    let claims_path = format!("/api/v1/projects/{project}/tasks/{task}/claims");
    let (_, current) = get(&w, &format!("/api/v1/projects/{project}/tasks/{task}")).await;
    let task_revision = current["revision"].as_i64().unwrap();
    let (status, _, body) = w
        .req(Method::POST, &claims_path)
        .bearer(&producer)
        .idempotency()
        .if_match(task_revision)
        .json(json!({ "phase": "review", "submission_id": submission_id }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "producer_cannot_review");

    // An agent review without the review claim is a validation failure.
    let (status, _, body) = w
        .req(Method::POST, &reviews)
        .bearer(&reviewer)
        .idempotency()
        .if_match(revision)
        .json(json!({ "decision": "approve", "reason": "no claim" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation_error");

    // A review claim naming an old submission while a newer one is pending
    // is 409 submission_superseded.
    let (status, _, withdrawn) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{project}/submissions/{submission_id}/withdraw"),
        )
        .idempotency()
        .if_match(revision)
        .json(json!({ "reason": "redoing it" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{withdrawn}");
    let replacement = agent_submission(&w, &project, &task, &producer).await;
    assert_ne!(replacement["id"], json!(submission_id));
    let (_, current) = get(&w, &format!("/api/v1/projects/{project}/tasks/{task}")).await;
    let (status, _, body) = w
        .req(Method::POST, &claims_path)
        .bearer(&reviewer)
        .idempotency()
        .if_match(current["revision"].as_i64().unwrap())
        .json(json!({ "phase": "review", "submission_id": submission_id }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "submission_superseded");
}

#[tokio::test]
async fn task_dependencies_crossing_epics_are_409_scope_mismatch() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let (_, _, other_epic) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/epics", h.project),
        )
        .idempotency()
        .json(json!({ "goal_id": h.goal, "title": "Other epic" }))
        .send(&w.app)
        .await;
    let (_, _, other_task) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks", h.project),
        )
        .idempotency()
        .json(json!({ "epic_id": other_epic["id"], "title": "Elsewhere", "type_key": "code" }))
        .send(&w.app)
        .await;

    let (status, _, body) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/dependencies", h.project),
        )
        .idempotency()
        .json(json!({
            "level": "task",
            "dependent_id": h.task,
            "prerequisite_id": other_task["id"],
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "scope_mismatch");
}

#[tokio::test]
async fn epic_graph_past_the_node_bound_is_422_graph_too_large() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    // Direct-SQL fixture for scale only (mirrors the core graph bound test):
    // 2001 command transactions would dominate the test.
    let mut tx = w.store.pool().begin().await.unwrap();
    for _ in 0..2_001 {
        sqlx::query(
            "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, epic_id, \
             title, description, type_key, status, phase, planning_required, plan_review, \
             work_review, archived, attempt_count) \
             VALUES (?1, 1, ?2, ?2, ?3, ?4, 'x', '', 'code', 'open', 'execution', 0, \
             'none', 'none', 0, 0)",
        )
        .bind(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind("2026-09-14T00:00:00.000Z")
        .bind(&h.project)
        .bind(&h.epic)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let (status, body) = get(
        &w,
        &format!("/api/v1/projects/{}/epics/{}/graph", h.project, h.epic),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "graph_too_large");
}

#[tokio::test]
async fn content_resource_reads_carry_quoted_revision_etags() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let (_, _, document) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/documents", h.project),
        )
        .idempotency()
        .json(json!({
            "owner_kind": "task",
            "owner_id": h.task,
            "kind": "note",
            "title": "Notes",
            "body": "n1",
            "links": [],
        }))
        .send(&w.app)
        .await;
    let document_id = document["id"].as_str().unwrap();
    let revision_id = document["latest_revision_id"].as_str().unwrap();

    let (_, headers, _) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/documents/{document_id}", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(headers[header::ETAG], "\"1\"");
    let (_, headers, _) = w
        .owner(
            Method::GET,
            &format!(
                "/api/v1/projects/{}/document-revisions/{revision_id}",
                h.project
            ),
        )
        .send(&w.app)
        .await;
    assert_eq!(headers[header::ETAG], "\"1\"");
}

#[tokio::test]
async fn document_create_replay_returns_the_original() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    let body = json!({
        "owner_kind": "task",
        "owner_id": h.task,
        "kind": "note",
        "title": "Replayed",
        "body": "once",
        "links": [],
    });

    let (status, _, first) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/documents", h.project),
        )
        .header("idempotency-key", &key)
        .json(body.clone())
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (status, _, replayed) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/documents", h.project),
        )
        .header("idempotency-key", &key)
        .json(body)
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{replayed}");
    assert_eq!(replayed["id"], first["id"], "replay returns the original");

    let (_, listed) = get(&w, &format!("/api/v1/projects/{}/documents", h.project)).await;
    assert_eq!(
        listed["items"].as_array().unwrap().len(),
        1,
        "no duplicate was committed"
    );
}
