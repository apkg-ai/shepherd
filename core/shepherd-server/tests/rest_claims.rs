// Claims, sessions, work and context over live HTTP: lease plumbing,
// idempotent replay for the replayable command family, and the advisory
// work list. Lease state machines are covered by the core v1_* suites.

mod common;

use axum::http::{Method, StatusCode};
use common::{World, seed_hierarchy, world};
use serde_json::json;

fn claims_path(project: &str, task: &str) -> String {
    format!("/api/v1/projects/{project}/tasks/{task}/claims")
}

async fn claim(w: &World, project: &str, task: &str, key: &str) -> serde_json::Value {
    // If-Match is the task's current revision (plan/07 claim preconditions).
    let (_, _, current) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{project}/tasks/{task}"),
        )
        .send(&w.app)
        .await;
    let revision = current["revision"].as_i64().unwrap();
    let (status, _, body) = w
        .owner(Method::POST, &claims_path(project, task))
        .header("idempotency-key", key)
        .if_match(revision)
        .json(json!({ "phase": "execute" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

fn fresh_key() -> String {
    uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
}

#[tokio::test]
async fn claim_grant_includes_context_and_replay_returns_original_lease_even_if_expired() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let key = fresh_key();

    let grant = claim(&w, &h.project, &h.task, &key).await;
    assert_eq!(grant["claim"]["status"], "active");
    assert_eq!(grant["claim"]["phase"], "execute");
    let lease = grant["lease_token"].as_str().unwrap().to_string();
    assert!(!lease.is_empty());
    // The grant carries the full context bundle (plan/07 ClaimGrant).
    assert_eq!(grant["context"]["task"]["id"], json!(h.task));
    assert_eq!(grant["context"]["epic"]["id"], json!(h.epic));
    assert_eq!(grant["context"]["project"]["id"], json!(h.project));
    assert_eq!(grant["context"]["truncated"], false);

    // Same key and body after expiry: the original lease comes back; the
    // client reads expires_at (plan/07).
    w.clock.advance(chrono::TimeDelta::hours(2));
    let replayed = claim(&w, &h.project, &h.task, &key).await;
    assert_eq!(replayed["lease_token"].as_str().unwrap(), lease);
    assert_eq!(replayed["claim"]["id"], grant["claim"]["id"]);

    // Same key, different body: 409 idempotency_conflict.
    let (status, _, body) = w
        .owner(Method::POST, &claims_path(&h.project, &h.task))
        .header("idempotency-key", &key)
        .if_match(1)
        .json(json!({ "phase": "execute", "ttl_seconds": 120 }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "idempotency_conflict");
}

#[tokio::test]
async fn renew_with_wrong_lease_is_409_lease_invalid_and_missing_lease_is_428() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let grant = claim(&w, &h.project, &h.task, &fresh_key()).await;
    let claim_id = grant["claim"]["id"].as_str().unwrap();
    let renew = format!("/api/v1/projects/{}/claims/{claim_id}/renew", h.project);

    let (status, _, body) = w
        .owner(Method::POST, &renew)
        .idempotency()
        .header("x-lease-token", "not-the-lease")
        .json(json!({}))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "lease_invalid");

    let (status, _, body) = w
        .owner(Method::POST, &renew)
        .idempotency()
        .json(json!({}))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["code"], "precondition_required");

    // The real lease renews and extends expiry.
    let lease = grant["lease_token"].as_str().unwrap();
    let (status, _, renewed) = w
        .owner(Method::POST, &renew)
        .idempotency()
        .header("x-lease-token", lease)
        .json(json!({ "ttl_seconds": 900 }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{renewed}");
    assert!(
        renewed["expires_at"].as_str().unwrap() > grant["claim"]["expires_at"].as_str().unwrap()
    );
}

#[tokio::test]
async fn report_uses_lease_not_task_revision() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let grant = claim(&w, &h.project, &h.task, &fresh_key()).await;
    let claim_id = grant["claim"]["id"].as_str().unwrap();
    let lease = grant["lease_token"].as_str().unwrap();

    // No If-Match anywhere: the lease token authorizes the report (plan/07).
    // A failed report needs no task-owned revisions (plan/03).
    let (status, _, session) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/claims/{claim_id}/report", h.project),
        )
        .idempotency()
        .header("x-lease-token", lease)
        .json(json!({
            "outcome": "failed",
            "summary": "ran out of context",
            "failure_reason": "needs another attempt",
            "document_revision_ids": [],
            "links": [],
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    assert_eq!(session["task_id"], json!(h.task));
    assert_eq!(session["outcome"], "failed");
    assert_eq!(session["phase"], "execute");

    // The spent lease no longer authorizes anything.
    let (status, _, body) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/claims/{claim_id}/release", h.project),
        )
        .idempotency()
        .header("x-lease-token", lease)
        .json(json!({}))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "lease_invalid");
}

#[tokio::test]
async fn list_work_is_advisory_and_eligibility_scoped() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let work = format!("/api/v1/projects/{}/work", h.project);

    // Planning is not required, so the plan queue is empty and the execute
    // queue offers the task.
    let (status, _, plan_queue) = w
        .owner(Method::GET, &format!("{work}?phase=plan"))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{plan_queue}");
    assert_eq!(plan_queue["items"], json!([]));

    let (_, _, execute_queue) = w
        .owner(Method::GET, &format!("{work}?phase=execute"))
        .send(&w.app)
        .await;
    assert_eq!(execute_queue["items"][0]["id"], json!(h.task));
    assert_eq!(
        execute_queue["items"][0]["eligibility"]["can_execute"],
        true
    );

    // Claiming re-checks eligibility: the claimed task leaves the queue.
    claim(&w, &h.project, &h.task, &fresh_key()).await;
    let (_, _, after_claim) = w
        .owner(Method::GET, &format!("{work}?phase=execute"))
        .send(&w.app)
        .await;
    assert_eq!(after_claim["items"], json!([]));

    // The phase parameter is required by the contract.
    let (status, _, body) = w.owner(Method::GET, &work).send(&w.app).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn task_sessions_http_newest_first() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;

    for summary in ["first", "second"] {
        let grant = claim(&w, &h.project, &h.task, &fresh_key()).await;
        let claim_id = grant["claim"]["id"].as_str().unwrap();
        let lease = grant["lease_token"].as_str().unwrap();
        let (status, _, body) = w
            .owner(
                Method::POST,
                &format!("/api/v1/projects/{}/claims/{claim_id}/report", h.project),
            )
            .idempotency()
            .header("x-lease-token", lease)
            .json(json!({
                "outcome": "failed",
                "summary": summary,
                "failure_reason": "retrying",
                "document_revision_ids": [],
                "links": [],
            }))
            .send(&w.app)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        w.clock.advance(chrono::TimeDelta::minutes(5));
    }

    let (status, _, sessions) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{}/sessions", h.project, h.task),
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{sessions}");
    let summaries: Vec<&str> = sessions["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["second", "first"], "newest first");
}

#[tokio::test]
async fn context_endpoint_returns_caller_scoped_bundle() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let (status, _, bundle) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{}/context", h.project, h.task),
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{bundle}");
    assert_eq!(bundle["task"]["id"], json!(h.task));
    assert_eq!(bundle["goal"]["id"], json!(h.goal));
    assert!(
        bundle["task"]["eligibility"]["can_execute"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(bundle["selected_plan"], json!([]));
    assert_eq!(bundle["truncated"], false);
    assert_eq!(bundle["continuation_paths"], json!([]));
}

#[tokio::test]
async fn context_over_budget_is_422_with_continuation() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    // Plan selection requires a planning task.
    let (status, _, planned) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks", h.project),
        )
        .idempotency()
        .json(json!({
            "epic_id": h.epic,
            "title": "Planned",
            "type_key": "code",
            "planning_required": true,
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{planned}");
    let task = planned["id"].as_str().unwrap().to_string();

    // A selected plan of 65,536 four-byte scalars serializes past the 256 KiB
    // context budget on its own (plan/05): mandatory fields + plan cannot fit.
    let body: String = "\u{1F980}".repeat(65_536);
    let (status, _, document) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/documents", h.project),
        )
        .idempotency()
        .json(json!({
            "owner_kind": "task",
            "owner_id": task,
            "kind": "plan",
            "title": "Oversized plan",
            "body": body,
            "links": [],
        }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{document}");
    let revision = document["latest_revision_id"].as_str().unwrap();

    let (_, _, current) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{task}", h.project),
        )
        .send(&w.app)
        .await;
    let (status, _, selected) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks/{task}/plan", h.project),
        )
        .idempotency()
        .if_match(current["revision"].as_i64().unwrap())
        .json(json!({ "document_revision_id": revision }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{selected}");

    let (status, _, body) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{task}/context", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "context_too_large");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("document-revisions"),
        "continuation path points at getDocumentRevision: {body}"
    );
}

#[tokio::test]
async fn claim_with_unassemblable_context_releases_the_lease() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    // Same oversized-selected-plan state as the context-budget test.
    let (_, _, planned) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks", h.project),
        )
        .idempotency()
        .json(json!({
            "epic_id": h.epic,
            "title": "Oversized",
            "type_key": "code",
            "planning_required": true,
        }))
        .send(&w.app)
        .await;
    let task = planned["id"].as_str().unwrap().to_string();
    let body: String = "\u{1F980}".repeat(65_536);
    let (_, _, document) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/documents", h.project),
        )
        .idempotency()
        .json(json!({
            "owner_kind": "task",
            "owner_id": task,
            "kind": "plan",
            "title": "Oversized plan",
            "body": body,
            "links": [],
        }))
        .send(&w.app)
        .await;
    let revision = document["latest_revision_id"].as_str().unwrap();
    let (_, _, current) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{task}", h.project),
        )
        .send(&w.app)
        .await;
    let (status, _, selected) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks/{task}/plan", h.project),
        )
        .idempotency()
        .if_match(current["revision"].as_i64().unwrap())
        .json(json!({ "document_revision_id": revision }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{selected}");

    // The claim commits, context assembly fails, and the handler releases the
    // lease so the task does not stay locked behind an undeliverable grant.
    let (status, _, body) = w
        .owner(
            Method::POST,
            &format!("/api/v1/projects/{}/tasks/{task}/claims", h.project),
        )
        .idempotency()
        .if_match(selected["revision"].as_i64().unwrap())
        .json(json!({ "phase": "plan" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "context_too_large");

    let (_, _, active) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/claims?active=true", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(active["items"], json!([]), "no lease stays active");
    let (_, _, all) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/claims", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(all["items"][0]["status"], "released");
}

#[tokio::test]
async fn corrupt_session_phase_is_integrity_failure_not_panic() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let mut conn = w.store.pool().acquire().await.unwrap();
    sqlx::raw_sql("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sessions (id, task_id, claim_id, actor_id, phase, started_at, ended_at, \
         outcome, summary, failure_reason, document_revision_ids, links) \
         VALUES (?1, ?2, ?3, ?4, 'review', ?5, ?5, 'succeeded', '', '', '[]', '[]')",
    )
    .bind(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
    .bind(&h.task)
    .bind(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
    .bind({
        let (_, _, principal) = w.owner(Method::GET, "/api/v1/principal").send(&w.app).await;
        principal["id"].as_str().unwrap().to_string()
    })
    .bind("2026-09-14T00:00:00.000Z")
    .execute(&mut *conn)
    .await
    .unwrap();
    drop(conn);

    let (status, _, body) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/tasks/{}/sessions", h.project, h.task),
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "integrity_failure");
}
