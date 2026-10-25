// Contract conformance harness: one pass through every implemented operation,
// validating each live response body against the served spec's schema for the
// returned status, then asserting coverage equals the catalog minus the
// operations deferred to the portability follow-up and the handwritten SSE
// route (added with the SSE commit).

mod common;

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode, header};
use common::{TestRequest, World, validate_schema_value, world};
use serde_json::{Value, json};

struct Tour {
    w: World,
    spec: Value,
    catalog: Value,
    covered: BTreeSet<String>,
}

impl Tour {
    async fn new() -> Self {
        Self {
            w: world().await,
            spec: common::load_spec(),
            catalog: serde_json::from_str(include_str!("../../../plan/contracts/operations.json"))
                .expect("operations catalog parses"),
            covered: BTreeSet::new(),
        }
    }

    fn template(&self, operation_id: &str) -> (String, String) {
        let op = self
            .catalog
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["operation_id"] == operation_id)
            .unwrap_or_else(|| panic!("{operation_id} not in catalog"));
        (
            op["method"].as_str().unwrap().to_string(),
            op["path"].as_str().unwrap().to_string(),
        )
    }

    /// Sends the request, asserts the status, and validates the body against
    /// the spec's declared schema for that operation and status.
    async fn call(
        &mut self,
        operation_id: &str,
        path: &str,
        expect: StatusCode,
        build: impl FnOnce(&World, Method, &str) -> TestRequest,
    ) -> Value {
        let (method_text, template) = self.template(operation_id);
        let method: Method = method_text.to_uppercase().parse().unwrap();
        let request = build(&self.w, method, path);
        let (status, _, body) = request.send(&self.w.app).await;
        assert_eq!(status, expect, "{operation_id} {path}: {body}");
        let schema = &self.spec["paths"][&template][&method_text]["responses"]
            [expect.as_u16().to_string()]["content"]["application/json"]["schema"];
        assert!(
            !schema.is_null(),
            "{operation_id} must declare a {expect} JSON response"
        );
        validate_schema_value(&self.spec, schema, &body, operation_id);
        self.covered.insert(operation_id.to_string());
        body
    }

    async fn owner_get(&mut self, operation_id: &str, path: &str) -> Value {
        self.call(operation_id, path, StatusCode::OK, |w, m, p| w.owner(m, p))
            .await
    }

    async fn owner_command(
        &mut self,
        operation_id: &str,
        path: &str,
        expect: StatusCode,
        revision: Option<i64>,
        body: Value,
    ) -> Value {
        self.call(operation_id, path, expect, |w, m, p| {
            let request = w.owner(m, p).idempotency().json(body);
            match revision {
                Some(revision) => request.if_match(revision),
                None => request,
            }
        })
        .await
    }
}

#[tokio::test]
async fn every_implemented_operation_conforms() {
    let mut t = Tour::new().await;

    // ── System and identity ─────────────────────────────────────────────
    t.call("getHealth", "/health", StatusCode::OK, |w, m, p| {
        w.req(m, p)
    })
    .await;
    // One raw login keeps the cookie+CSRF pair for the session reads; the
    // validated createBrowserSession call below opens a second session (one
    // human actor can hold several, plan/12).
    let (status, headers, raw_login) =
        t.w.req(Method::POST, "/api/v1/session")
            .header("origin", "http://127.0.0.1:7437")
            .json(json!({ "owner_token": t.w.owner_token }))
            .send(&t.w.app)
            .await;
    assert_eq!(status, StatusCode::CREATED);
    let cookie = headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = raw_login["csrf_token"].as_str().unwrap().to_string();
    let owner_token = t.w.owner_token.clone();
    t.call(
        "createBrowserSession",
        "/api/v1/session",
        StatusCode::CREATED,
        |w, m, p| {
            w.req(m, p)
                .header("origin", "http://127.0.0.1:7437")
                .json(json!({ "owner_token": owner_token }))
        },
    )
    .await;
    let (cookie_header, csrf_header) = (cookie.clone(), csrf.clone());
    t.call(
        "getBrowserSession",
        "/api/v1/session",
        StatusCode::OK,
        |w, m, p| w.req(m, p).header("cookie", &cookie_header),
    )
    .await;
    t.owner_get("getPrincipal", "/api/v1/principal").await;
    let agent = t
        .owner_command(
            "createAgent",
            "/api/v1/agents",
            StatusCode::CREATED,
            None,
            json!({ "label": "tour-agent" }),
        )
        .await;
    let agent_token = agent["token"].as_str().unwrap().to_string();
    let agent_id = agent["actor"]["id"].as_str().unwrap().to_string();
    t.owner_get("listAgents", "/api/v1/agents").await;

    // ── Projects ────────────────────────────────────────────────────────
    let project = t
        .owner_command(
            "createProject",
            "/api/v1/projects",
            StatusCode::CREATED,
            None,
            json!({ "name": "Tour" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get("listProjects", "/api/v1/projects").await;
    t.owner_get("getProject", &format!("/api/v1/projects/{project}"))
        .await;
    t.owner_command(
        "updateProject",
        &format!("/api/v1/projects/{project}"),
        StatusCode::OK,
        Some(1),
        json!({ "description": "guided tour" }),
    )
    .await;

    // ── Task types ──────────────────────────────────────────────────────
    let task_types = format!("/api/v1/projects/{project}/task-types");
    t.owner_get("listTaskTypes", &task_types).await;
    let task_type = t
        .owner_command(
            "createTaskType",
            &task_types,
            StatusCode::CREATED,
            None,
            json!({ "key": "tour_pass", "label": "Tour pass" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_command(
        "updateTaskType",
        &format!("{task_types}/{task_type}"),
        StatusCode::OK,
        Some(1),
        json!({ "label": "Tour sweep" }),
    )
    .await;

    // ── Goals ───────────────────────────────────────────────────────────
    let goals = format!("/api/v1/projects/{project}/goals");
    let goal = t
        .owner_command(
            "createGoal",
            &goals,
            StatusCode::CREATED,
            None,
            json!({ "title": "Goal" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get("listGoals", &goals).await;
    t.owner_get("getGoal", &format!("{goals}/{goal}")).await;
    t.owner_command(
        "updateGoal",
        &format!("{goals}/{goal}"),
        StatusCode::OK,
        Some(1),
        json!({ "description": "the north star" }),
    )
    .await;

    // ── Epics ───────────────────────────────────────────────────────────
    let epics = format!("/api/v1/projects/{project}/epics");
    let epic = t
        .owner_command(
            "createEpic",
            &epics,
            StatusCode::CREATED,
            None,
            json!({ "goal_id": goal, "title": "Main epic" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get("listEpics", &epics).await;
    t.owner_get("getEpic", &format!("{epics}/{epic}")).await;
    t.owner_command(
        "updateEpic",
        &format!("{epics}/{epic}"),
        StatusCode::OK,
        Some(1),
        json!({ "description": "carries the tour tasks" }),
    )
    .await;
    let blocked = t
        .owner_command(
            "blockEpic",
            &format!("{epics}/{epic}/block"),
            StatusCode::OK,
            Some(2),
            json!({ "reason": "tour stop" }),
        )
        .await;
    t.owner_command(
        "unblockEpic",
        &format!("{epics}/{epic}/unblock"),
        StatusCode::OK,
        Some(blocked["revision"].as_i64().unwrap()),
        json!({}),
    )
    .await;

    // The proposal path: an agent-created epic is accepted by the owner.
    let agent_token_clone = agent_token.clone();
    let goal_clone = goal.clone();
    let proposed_epic = t
        .call("createEpic", &epics, StatusCode::CREATED, move |w, m, p| {
            w.req(m, p)
                .bearer(&agent_token_clone)
                .idempotency()
                .json(json!({ "goal_id": goal_clone, "title": "Proposed epic" }))
        })
        .await;
    assert_eq!(proposed_epic["status"], "proposed");
    let proposed_epic_id = proposed_epic["id"].as_str().unwrap().to_string();
    t.owner_command(
        "acceptEpic",
        &format!("{epics}/{proposed_epic_id}/accept"),
        StatusCode::OK,
        Some(proposed_epic["revision"].as_i64().unwrap()),
        json!({}),
    )
    .await;
    // Spare epics for terminal transitions.
    let cancel_epic = t
        .owner_command(
            "createEpic",
            &epics,
            StatusCode::CREATED,
            None,
            json!({ "goal_id": goal, "title": "To cancel" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled_epic = t
        .owner_command(
            "cancelEpic",
            &format!("{epics}/{cancel_epic}/cancel"),
            StatusCode::OK,
            Some(1),
            json!({ "reason": "tour detour" }),
        )
        .await;
    t.owner_command(
        "archiveEpic",
        &format!("{epics}/{cancel_epic}/archive"),
        StatusCode::OK,
        Some(cancelled_epic["revision"].as_i64().unwrap()),
        json!({ "reason": "tour cleanup" }),
    )
    .await;
    let complete_epic = t
        .owner_command(
            "createEpic",
            &epics,
            StatusCode::CREATED,
            None,
            json!({ "goal_id": goal, "title": "To complete" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_command(
        "completeEpic",
        &format!("{epics}/{complete_epic}/complete"),
        StatusCode::OK,
        Some(1),
        json!({ "reason": "explicitly empty" }),
    )
    .await;

    // ── Tasks ───────────────────────────────────────────────────────────
    let tasks = format!("/api/v1/projects/{project}/tasks");
    let task = t
        .owner_command(
            "createTask",
            &tasks,
            StatusCode::CREATED,
            None,
            json!({ "epic_id": epic, "title": "Main task", "type_key": "code" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get("listTasks", &tasks).await;
    t.owner_get("getTask", &format!("{tasks}/{task}")).await;
    t.owner_command(
        "updateTask",
        &format!("{tasks}/{task}"),
        StatusCode::OK,
        Some(1),
        json!({ "description": "the tour workhorse" }),
    )
    .await;
    let proposed_task = t
        .call("createTask", &tasks, StatusCode::CREATED, |w, m, p| {
            w.req(m, p)
                .bearer(&agent_token)
                .idempotency()
                .json(json!({ "epic_id": epic, "title": "Proposed task", "type_key": "code" }))
        })
        .await;
    let proposed_task_id = proposed_task["id"].as_str().unwrap().to_string();
    let accepted = t
        .owner_command(
            "acceptTask",
            &format!("{tasks}/{proposed_task_id}/accept"),
            StatusCode::OK,
            Some(proposed_task["revision"].as_i64().unwrap()),
            json!({}),
        )
        .await;
    let blocked = t
        .owner_command(
            "blockTask",
            &format!("{tasks}/{proposed_task_id}/block"),
            StatusCode::OK,
            Some(accepted["revision"].as_i64().unwrap()),
            json!({ "reason": "tour stop" }),
        )
        .await;
    t.owner_command(
        "unblockTask",
        &format!("{tasks}/{proposed_task_id}/unblock"),
        StatusCode::OK,
        Some(blocked["revision"].as_i64().unwrap()),
        json!({}),
    )
    .await;
    let cancel_task = t
        .owner_command(
            "createTask",
            &tasks,
            StatusCode::CREATED,
            None,
            json!({ "epic_id": epic, "title": "To cancel", "type_key": "code" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled = t
        .owner_command(
            "cancelTask",
            &format!("{tasks}/{cancel_task}/cancel"),
            StatusCode::OK,
            Some(1),
            json!({ "reason": "tour detour" }),
        )
        .await;
    // Waiving marks a cancelled task deliberately skipped (plan/03).
    let waived = t
        .owner_command(
            "waiveTask",
            &format!("{tasks}/{cancel_task}/waive"),
            StatusCode::OK,
            Some(cancelled["revision"].as_i64().unwrap()),
            json!({ "reason": "not needed for the tour" }),
        )
        .await;
    t.owner_command(
        "archiveTask",
        &format!("{tasks}/{cancel_task}/archive"),
        StatusCode::OK,
        Some(waived["revision"].as_i64().unwrap()),
        json!({ "reason": "tour cleanup" }),
    )
    .await;

    // ── Dependencies and graphs ─────────────────────────────────────────
    let dependencies = format!("/api/v1/projects/{project}/dependencies");
    let dependency = t
        .owner_command(
            "createDependency",
            &dependencies,
            StatusCode::CREATED,
            None,
            json!({ "level": "task", "dependent_id": proposed_task_id, "prerequisite_id": task }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get("listDependencies", &dependencies).await;
    t.call(
        "deleteDependency",
        &format!("{dependencies}/{dependency}"),
        StatusCode::OK,
        |w, m, p| w.owner(m, p).idempotency().if_match(1),
    )
    .await;
    t.owner_get("getGoalGraph", &format!("{goals}/{goal}/graph"))
        .await;
    t.owner_get("getEpicGraph", &format!("{epics}/{epic}/graph"))
        .await;

    // ── Work, claims, sessions ──────────────────────────────────────────
    t.owner_get(
        "listWork",
        &format!("/api/v1/projects/{project}/work?phase=execute"),
    )
    .await;
    t.owner_get("getTaskContext", &format!("{tasks}/{task}/context"))
        .await;
    let task_revision = t.owner_get("getTask", &format!("{tasks}/{task}")).await["revision"]
        .as_i64()
        .unwrap();
    let grant = t
        .owner_command(
            "claimTask",
            &format!("{tasks}/{task}/claims"),
            StatusCode::CREATED,
            Some(task_revision),
            json!({ "phase": "execute" }),
        )
        .await;
    let claim_id = grant["claim"]["id"].as_str().unwrap().to_string();
    let lease = grant["lease_token"].as_str().unwrap().to_string();
    t.owner_get("listClaims", &format!("/api/v1/projects/{project}/claims"))
        .await;
    let claims = format!("/api/v1/projects/{project}/claims");
    let renew_lease = lease.clone();
    t.call(
        "renewClaim",
        &format!("{claims}/{claim_id}/renew"),
        StatusCode::OK,
        |w, m, p| {
            w.owner(m, p)
                .idempotency()
                .header("x-lease-token", &renew_lease)
                .json(json!({ "ttl_seconds": 600 }))
        },
    )
    .await;
    let release_lease = lease.clone();
    t.call(
        "releaseClaim",
        &format!("{claims}/{claim_id}/release"),
        StatusCode::OK,
        |w, m, p| {
            w.owner(m, p)
                .idempotency()
                .header("x-lease-token", &release_lease)
                .json(json!({}))
        },
    )
    .await;

    // ── Documents ───────────────────────────────────────────────────────
    let documents = format!("/api/v1/projects/{project}/documents");
    let finding = t
        .owner_command(
            "createDocument",
            &documents,
            StatusCode::CREATED,
            None,
            json!({
                "owner_kind": "task",
                "owner_id": task,
                "kind": "finding",
                "title": "Findings",
                "body": "initial findings",
                "links": [],
            }),
        )
        .await;
    let document_id = finding["id"].as_str().unwrap().to_string();
    t.owner_get("listDocuments", &documents).await;
    t.owner_get("getDocument", &format!("{documents}/{document_id}"))
        .await;
    let revision = t
        .owner_command(
            "createDocumentRevision",
            &format!("{documents}/{document_id}/revisions"),
            StatusCode::CREATED,
            Some(1),
            json!({ "body": "refined findings", "links": [] }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_get(
        "listDocumentRevisions",
        &format!("{documents}/{document_id}/revisions"),
    )
    .await;
    t.owner_get(
        "getDocumentRevision",
        &format!("/api/v1/projects/{project}/document-revisions/{revision}"),
    )
    .await;

    // ── Report → submission → review/withdraw ───────────────────────────
    let task_revision = t.owner_get("getTask", &format!("{tasks}/{task}")).await["revision"]
        .as_i64()
        .unwrap();
    let grant = t
        .owner_command(
            "claimTask",
            &format!("{tasks}/{task}/claims"),
            StatusCode::CREATED,
            Some(task_revision),
            json!({ "phase": "execute" }),
        )
        .await;
    let claim_id = grant["claim"]["id"].as_str().unwrap().to_string();
    let report_lease = grant["lease_token"].as_str().unwrap().to_string();
    t.call(
        "reportClaim",
        &format!("{claims}/{claim_id}/report"),
        StatusCode::CREATED,
        |w, m, p| {
            w.owner(m, p)
                .idempotency()
                .header("x-lease-token", &report_lease)
                .json(json!({
                    "outcome": "succeeded",
                    "summary": "tour complete",
                    "document_revision_ids": [revision],
                    "links": [],
                }))
        },
    )
    .await;
    t.owner_get("listTaskSessions", &format!("{tasks}/{task}/sessions"))
        .await;
    let submissions = format!("/api/v1/projects/{project}/submissions");
    let listed = t.owner_get("listSubmissions", &submissions).await;
    let submission = listed["items"][0]["id"].as_str().unwrap().to_string();
    let submission_revision = listed["items"][0]["revision"].as_i64().unwrap();
    t.owner_get("getSubmission", &format!("{submissions}/{submission}"))
        .await;
    t.owner_command(
        "reviewSubmission",
        &format!("{submissions}/{submission}/reviews"),
        StatusCode::CREATED,
        Some(submission_revision),
        json!({ "decision": "approve", "reason": "looks right" }),
    )
    .await;
    t.owner_get(
        "listSubmissionReviews",
        &format!("{submissions}/{submission}/reviews"),
    )
    .await;

    // A second cycle produces a submission to withdraw.
    let accepted_task = t
        .owner_get("getTask", &format!("{tasks}/{proposed_task_id}"))
        .await;
    let grant = t
        .owner_command(
            "claimTask",
            &format!("{tasks}/{proposed_task_id}/claims"),
            StatusCode::CREATED,
            Some(accepted_task["revision"].as_i64().unwrap()),
            json!({ "phase": "execute" }),
        )
        .await;
    let claim_id = grant["claim"]["id"].as_str().unwrap().to_string();
    let withdraw_lease = grant["lease_token"].as_str().unwrap().to_string();
    let second_doc = t
        .owner_command(
            "createDocument",
            &documents,
            StatusCode::CREATED,
            None,
            json!({
                "owner_kind": "task",
                "owner_id": proposed_task_id,
                "kind": "handoff",
                "title": "Handoff",
                "body": "handoff notes",
                "links": [],
            }),
        )
        .await;
    let second_revision = second_doc["latest_revision_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _, _) =
        t.w.owner(Method::POST, &format!("{claims}/{claim_id}/report"))
            .idempotency()
            .header("x-lease-token", &withdraw_lease)
            .json(json!({
                "outcome": "succeeded",
                "summary": "ready for withdraw",
                "document_revision_ids": [second_revision],
                "links": [],
            }))
            .send(&t.w.app)
            .await;
    assert_eq!(status, StatusCode::CREATED);
    let pending = t
        .owner_get("listSubmissions", &format!("{submissions}?status=pending"))
        .await;
    let second_submission = pending["items"][0]["id"].as_str().unwrap().to_string();
    let second_submission_revision = pending["items"][0]["revision"].as_i64().unwrap();
    t.owner_command(
        "withdrawSubmission",
        &format!("{submissions}/{second_submission}/withdraw"),
        StatusCode::OK,
        Some(second_submission_revision),
        json!({ "reason": "superseded by the first" }),
    )
    .await;

    // ── Plan selection ──────────────────────────────────────────────────
    let planned_task = t
        .owner_command(
            "createTask",
            &tasks,
            StatusCode::CREATED,
            None,
            json!({
                "epic_id": epic,
                "title": "Planned task",
                "type_key": "code",
                "planning_required": true,
            }),
        )
        .await;
    let planned_task_id = planned_task["id"].as_str().unwrap().to_string();
    let plan_doc = t
        .owner_command(
            "createDocument",
            &documents,
            StatusCode::CREATED,
            None,
            json!({
                "owner_kind": "task",
                "owner_id": planned_task_id,
                "kind": "plan",
                "title": "Plan",
                "body": "the plan",
                "links": [],
            }),
        )
        .await;
    let plan_revision = plan_doc["latest_revision_id"].as_str().unwrap().to_string();
    let planned_revision = t
        .owner_get("getTask", &format!("{tasks}/{planned_task_id}"))
        .await["revision"]
        .as_i64()
        .unwrap();
    t.owner_command(
        "selectTaskPlan",
        &format!("{tasks}/{planned_task_id}/plan"),
        StatusCode::OK,
        Some(planned_revision),
        json!({ "document_revision_id": plan_revision }),
    )
    .await;

    // ── History, archival tails, identity teardown ──────────────────────
    t.owner_get(
        "listHistory",
        &format!("/api/v1/projects/{project}/history"),
    )
    .await;
    let spare_goal = t
        .owner_command(
            "createGoal",
            &goals,
            StatusCode::CREATED,
            None,
            json!({ "title": "Spare goal" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_command(
        "archiveGoal",
        &format!("{goals}/{spare_goal}/archive"),
        StatusCode::OK,
        Some(1),
        json!({ "reason": "tour cleanup" }),
    )
    .await;
    let spare_project = t
        .owner_command(
            "createProject",
            "/api/v1/projects",
            StatusCode::CREATED,
            None,
            json!({ "name": "Spare" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.owner_command(
        "archiveProject",
        &format!("/api/v1/projects/{spare_project}/archive"),
        StatusCode::OK,
        Some(1),
        json!({ "reason": "tour cleanup" }),
    )
    .await;
    t.owner_command(
        "revokeAgent",
        &format!("/api/v1/agents/{agent_id}/revoke"),
        StatusCode::OK,
        None,
        json!({ "reason": "tour over" }),
    )
    .await;
    t.call(
        "deleteBrowserSession",
        "/api/v1/session",
        StatusCode::OK,
        |w, m, p| {
            w.req(m, p)
                .header("cookie", &cookie)
                .header("origin", "http://127.0.0.1:7437")
                .header("x-csrf-token", &csrf_header)
        },
    )
    .await;

    // ── Completeness: catalog minus deferred portability minus SSE ──────
    let deferred = ["exportProject", "importProject", "getEvents"];
    let expected: BTreeSet<String> = t
        .catalog
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["operation_id"].as_str().unwrap().to_string())
        .filter(|id| !deferred.contains(&id.as_str()))
        .collect();
    let missing: Vec<&String> = expected.difference(&t.covered).collect();
    assert!(missing.is_empty(), "operations not exercised: {missing:?}");
    assert_eq!(t.covered.len(), 67, "all generated operations covered");
}
