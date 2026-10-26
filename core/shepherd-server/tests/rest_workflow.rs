// Epics, tasks, dependencies and graphs over live HTTP: transition wiring,
// workflow-conflict error mapping and graph reads. Domain state machines are
// covered by the core v1_* suites.

mod common;

use axum::http::{Method, StatusCode};
use common::{World, world};
use serde_json::{Value, json};
use shepherd_core::storage::testing;

struct Seeded {
    w: World,
    project: String,
    goal: String,
    epic: String,
    task_a: String,
    task_b: String,
}

async fn post(w: &World, path: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, body) = w
        .owner(Method::POST, path)
        .idempotency()
        .json(body)
        .send(&w.app)
        .await;
    (status, body)
}

async fn seeded() -> Seeded {
    let w = world().await;
    let (status, project) = post(&w, "/api/v1/projects", json!({ "name": "P" })).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project = project["id"].as_str().unwrap().to_string();
    let (status, goal) = post(
        &w,
        &format!("/api/v1/projects/{project}/goals"),
        json!({ "title": "G" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{goal}");
    let goal = goal["id"].as_str().unwrap().to_string();
    let (status, epic) = post(
        &w,
        &format!("/api/v1/projects/{project}/epics"),
        json!({ "goal_id": goal, "title": "E" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{epic}");
    let epic = epic["id"].as_str().unwrap().to_string();
    let mut tasks = Vec::new();
    for title in ["A", "B"] {
        let (status, task) = post(
            &w,
            &format!("/api/v1/projects/{project}/tasks"),
            json!({ "epic_id": epic, "title": title, "type_key": "code" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        tasks.push(task["id"].as_str().unwrap().to_string());
    }
    Seeded {
        w,
        project,
        goal,
        epic,
        task_a: tasks.remove(0),
        task_b: tasks.remove(0),
    }
}

#[tokio::test]
async fn created_task_carries_eligibility_and_empty_plan_pointers() {
    let s = seeded().await;
    let (status, _, task) =
        s.w.owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{}", s.project, s.task_a),
        )
        .send(&s.w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["status"], "open");
    assert_eq!(task["phase"], "execution");
    assert_eq!(task["eligibility"]["can_execute"], true, "{task}");
    assert_eq!(task["selected_plan_revision_ids"], json!([]));
    assert_eq!(task["accepted_plan_submission_ids"], json!([]));
}

#[tokio::test]
async fn block_and_unblock_persist_reasons_and_flip_eligibility() {
    let s = seeded().await;
    let path = format!("/api/v1/projects/{}/tasks/{}", s.project, s.task_a);

    let (status, _, blocked) =
        s.w.owner(Method::POST, &format!("{path}/block"))
            .idempotency()
            .if_match(1)
            .json(json!({ "reason": "waiting on infra" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{blocked}");
    assert_eq!(blocked["blocks"][0]["reason"], "waiting on infra");
    assert_eq!(blocked["eligibility"]["can_execute"], false);

    let revision = blocked["revision"].as_i64().unwrap();
    let (status, _, unblocked) =
        s.w.owner(Method::POST, &format!("{path}/unblock"))
            .idempotency()
            .if_match(revision)
            .json(json!({}))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{unblocked}");
    assert_eq!(unblocked["blocks"], json!([]));
    assert_eq!(unblocked["eligibility"]["can_execute"], true);
}

#[tokio::test]
async fn dependency_cycle_is_409_and_leaves_state_unchanged() {
    let s = seeded().await;
    let path = format!("/api/v1/projects/{}/dependencies", s.project);

    let (status, created) = post(
        &s.w,
        &path,
        json!({ "level": "task", "dependent_id": s.task_b, "prerequisite_id": s.task_a }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, conflict) = post(
        &s.w,
        &path,
        json!({ "level": "task", "dependent_id": s.task_a, "prerequisite_id": s.task_b }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "dependency_cycle");

    // The failed mutation committed nothing: still exactly one dependency.
    let (_, _, listed) = s.w.owner(Method::GET, &path).send(&s.w.app).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn cross_project_dependency_is_rejected() {
    let s = seeded().await;
    let (_, other_project) = post(&s.w, "/api/v1/projects", json!({ "name": "Other" })).await;
    let other_project = other_project["id"].as_str().unwrap();
    let (_, other_goal) = post(
        &s.w,
        &format!("/api/v1/projects/{other_project}/goals"),
        json!({ "title": "OG" }),
    )
    .await;
    let (_, other_epic) = post(
        &s.w,
        &format!("/api/v1/projects/{other_project}/epics"),
        json!({ "goal_id": other_goal["id"], "title": "OE" }),
    )
    .await;
    let (_, other_task) = post(
        &s.w,
        &format!("/api/v1/projects/{other_project}/tasks"),
        json!({ "epic_id": other_epic["id"], "title": "OT", "type_key": "code" }),
    )
    .await;

    let (status, body) = post(
        &s.w,
        &format!("/api/v1/projects/{}/dependencies", s.project),
        json!({
            "level": "task",
            "dependent_id": s.task_a,
            "prerequisite_id": other_task["id"],
        }),
    )
    .await;
    // Identifiers in a route must resolve to its project (plan/07): 404.
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "not_found");
}

#[tokio::test]
async fn active_work_blocks_edits_and_leaves_revision_unchanged() {
    let s = seeded().await;
    // Claims are not servable until their commit lands; seed the fixture row.
    testing::seed_claim(
        s.w.store.pool(),
        shepherd_core::model::TaskId::from_uuid(s.task_a.parse().unwrap()),
        owner_id(&s.w).await,
        "execute",
        "active",
        "2026-09-14T00:00:00.000Z",
        "2027-01-01T00:00:00.000Z",
    )
    .await;

    let task_path = format!("/api/v1/projects/{}/tasks/{}", s.project, s.task_a);
    let (status, _, body) =
        s.w.owner(Method::PATCH, &task_path)
            .idempotency()
            .if_match(1)
            .json(json!({ "description": "rewrite while claimed" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "active_work");

    // The failed mutation did not move the revision.
    let (_, headers, _) = s.w.owner(Method::GET, &task_path).send(&s.w.app).await;
    assert_eq!(headers[axum::http::header::ETAG], "\"1\"");
}

async fn owner_id(w: &World) -> shepherd_core::model::ActorId {
    // The owner is the only human actor; resolve it through the public principal route.
    let (_, _, principal) = w.owner(Method::GET, "/api/v1/principal").send(&w.app).await;
    shepherd_core::model::ActorId::from_uuid(principal["id"].as_str().unwrap().parse().unwrap())
}

#[tokio::test]
async fn terminal_resources_reject_transitions_with_409() {
    let s = seeded().await;
    let path = format!("/api/v1/projects/{}/tasks/{}", s.project, s.task_a);
    let (status, _, cancelled) =
        s.w.owner(Method::POST, &format!("{path}/cancel"))
            .idempotency()
            .if_match(1)
            .json(json!({ "reason": "obsolete" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["cancellations"][0]["reason"], "obsolete");

    let revision = cancelled["revision"].as_i64().unwrap();
    let (status, _, body) =
        s.w.owner(Method::POST, &format!("{path}/block"))
            .idempotency()
            .if_match(revision)
            .json(json!({ "reason": "too late" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "terminal");
}

#[tokio::test]
async fn graphs_return_scoped_nodes_with_eligibility() {
    let s = seeded().await;
    post(
        &s.w,
        &format!("/api/v1/projects/{}/dependencies", s.project),
        json!({ "level": "task", "dependent_id": s.task_b, "prerequisite_id": s.task_a }),
    )
    .await;

    let (status, _, goal_graph) =
        s.w.owner(
            Method::GET,
            &format!("/api/v1/projects/{}/goals/{}/graph", s.project, s.goal),
        )
        .send(&s.w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{goal_graph}");
    assert_eq!(goal_graph["level"], "epic");
    assert_eq!(goal_graph["nodes"][0]["id"], json!(s.epic));
    assert_eq!(goal_graph["nodes"][0]["counts"]["total"], 2);
    assert!(goal_graph["nodes"][0]["eligibility"].is_object());

    let (status, _, epic_graph) =
        s.w.owner(
            Method::GET,
            &format!("/api/v1/projects/{}/epics/{}/graph", s.project, s.epic),
        )
        .send(&s.w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{epic_graph}");
    assert_eq!(epic_graph["level"], "task");
    assert_eq!(epic_graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(epic_graph["dependencies"].as_array().unwrap().len(), 1);
    assert_eq!(
        epic_graph["dependencies"][0]["dependent_id"],
        json!(s.task_b)
    );
}

#[tokio::test]
async fn epic_lifecycle_transitions_round_trip() {
    let s = seeded().await;
    let path = format!("/api/v1/projects/{}/epics/{}", s.project, s.epic);

    let (status, _, blocked) =
        s.w.owner(Method::POST, &format!("{path}/block"))
            .idempotency()
            .if_match(1)
            .json(json!({ "reason": "paused" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{blocked}");
    assert_eq!(blocked["blocks"][0]["reason"], "paused");

    let revision = blocked["revision"].as_i64().unwrap();
    let (status, _, unblocked) =
        s.w.owner(Method::POST, &format!("{path}/unblock"))
            .idempotency()
            .if_match(revision)
            .json(json!({}))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{unblocked}");
    assert_eq!(unblocked["blocks"], json!([]));

    let revision = unblocked["revision"].as_i64().unwrap();
    let (status, _, body) =
        s.w.owner(Method::POST, &format!("{path}/cancel"))
            .idempotency()
            .if_match(revision)
            .json(json!({ "reason": "descoped" }))
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "cancelled");
    assert_eq!(body["eligibility"]["can_execute"], false);
}

// Task-family replays serve the fresh eligibility view over the sealed id
// (the wire Task body carries caller-dependent fields the sealed core value
// cannot), so a retry after state moved on still names the same row.
#[tokio::test]
async fn create_task_retry_replays_the_same_row_across_state_changes() {
    let s = seeded().await;
    let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    let path = format!("/api/v1/projects/{}/tasks", s.project);
    let body = json!({ "epic_id": s.epic, "title": "C", "type_key": "code" });

    let (status, _, first) =
        s.w.owner(Method::POST, &path)
            .header("idempotency-key", &key)
            .json(body.clone())
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let task = first["id"].as_str().unwrap().to_string();

    // State moves on: the task is blocked before the caller retries.
    let (status, _, blocked) =
        s.w.owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks/{task}/block", s.project),
        )
        .idempotency()
        .if_match(first["revision"].as_i64().unwrap())
        .json(json!({ "reason": "paused" }))
        .send(&s.w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{blocked}");

    let (status, _, retry) =
        s.w.owner(Method::POST, &path)
            .header("idempotency-key", &key)
            .json(body)
            .send(&s.w.app)
            .await;
    assert_eq!(status, StatusCode::CREATED, "{retry}");
    assert_eq!(retry["id"], json!(task), "the retry names the same row");
    assert_eq!(retry["blocks"].as_array().unwrap().len(), 1, "fresh view");
    assert_eq!(retry["eligibility"]["can_execute"], false);

    let (status, _, listed) =
        s.w.owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks?limit=100", s.project),
        )
        .send(&s.w.app)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["items"].as_array().unwrap().len(), 3, "no duplicate");
}
