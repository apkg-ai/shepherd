// Contract tests for the shared Rust REST client: live tests run the real
// server on a loopback socket; capture tests assert the exact wire bytes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use shepherd_client::{
    CallOptions, Client, Error, EventStreamItem, ListClaimsQuery, ListTasksQuery, PageQuery,
    Secret, load_token_file, types,
};
use shepherd_core::commands::IdentityPaths;
use shepherd_core::storage::{open, testing};
use uuid::Uuid;

struct LiveServer {
    _dir: tempfile::TempDir,
    base_url: String,
    token_path: PathBuf,
}

async fn live_server() -> LiveServer {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = Arc::new(
        open(testing::store_options(dir.path(), "shepherd.db", clock))
            .await
            .unwrap(),
    );
    store
        .ensure_owner(&IdentityPaths::new(dir.path()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state =
        shepherd_server::AppState::new(store, shepherd_server::AuthConfig::new(port, false));
    let app = shepherd_server::router(state, "does-not-exist");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    LiveServer {
        token_path: dir.path().join("owner-token"),
        _dir: dir,
        base_url: format!("http://127.0.0.1:{port}"),
    }
}

impl LiveServer {
    fn client(&self) -> Client {
        Client::from_token_file(&self.base_url, &self.token_path).unwrap()
    }
}

fn keyed() -> CallOptions {
    CallOptions::new().idempotency_key(Uuid::now_v7())
}

struct Hierarchy {
    project: types::Project,
    epic: types::Epic,
    task: types::Task,
}

/// Owner-created project -> goal -> epic -> one executable task, all through
/// the client's public typed methods.
async fn seed_hierarchy(client: &Client) -> Hierarchy {
    let project = client
        .create_project(&types::ProjectCreate::new("P".into()), &keyed())
        .await
        .unwrap();
    let goal = client
        .create_goal(project.id, &types::GoalCreate::new("G".into()), &keyed())
        .await
        .unwrap();
    let epic = client
        .create_epic(
            project.id,
            &types::EpicCreate::new(goal.id, "E".into()),
            &keyed(),
        )
        .await
        .unwrap();
    let task = client
        .create_task(
            project.id,
            &types::TaskCreate::new(epic.id, "T".into(), "code".into()),
            &keyed(),
        )
        .await
        .unwrap();
    Hierarchy {
        project,
        epic,
        task,
    }
}

async fn claim_execute(client: &Client, h: &Hierarchy) -> types::ClaimGrant {
    let task = client.get_task(h.project.id, h.task.id).await.unwrap();
    client
        .claim_task(
            h.project.id,
            h.task.id,
            &types::ClaimInput::new(types::ClaimInputPhase::Execute),
            &keyed().if_match(task.revision as u64),
        )
        .await
        .unwrap()
}

fn failed_report() -> types::ReportInput {
    let mut report = types::ReportInput::new(
        Vec::new(),
        Vec::new(),
        types::ReportInputOutcome::Failed,
        "ran out of context".into(),
    );
    report.failure_reason = Some("needs another attempt".into());
    report
}

fn api_error(err: Error) -> types::Problem {
    match err {
        Error::Api(problem) => *problem,
        other => panic!("expected Api error, got {other:?}"),
    }
}

// ── Scripted capture server for exact-wire and retry assertions ─────────

struct CaptureServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    accepted: Arc<AtomicUsize>,
}

impl CaptureServer {
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// One scripted entry per accepted connection: `Some(response)` reads the
/// request, records it and replies; `None` drops the connection unread.
async fn capture_server(script: Vec<Option<String>>) -> CaptureServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let accepted = Arc::new(AtomicUsize::new(0));
    let recorded = requests.clone();
    let count = accepted.clone();
    tokio::spawn(async move {
        let mut script = script.into_iter();
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            count.fetch_add(1, Ordering::SeqCst);
            match script.next().unwrap_or(None) {
                None => drop(socket),
                Some(response) => {
                    use tokio::io::AsyncWriteExt;
                    let raw = read_request(&mut socket).await;
                    recorded.lock().unwrap().push(raw);
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            }
        }
    });
    CaptureServer {
        base_url,
        requests,
        accepted,
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            while buf.len() < end + 4 + content_length {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            return String::from_utf8_lossy(&buf).to_string();
        }
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            return String::from_utf8_lossy(&buf).to_string();
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn http_response(status_line: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status_line}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn problem_json(status: u16, code: &str) -> String {
    serde_json::json!({
        "type": format!("urn:shepherd:error:{code}"),
        "title": code,
        "status": status,
        "code": code,
        "detail": "scripted",
        "request_id": "00000000-0000-0000-0000-000000000000",
    })
    .to_string()
}

fn problem_response(status_line: &str, code: &str) -> String {
    let status: u16 = status_line.split(' ').next().unwrap().parse().unwrap();
    http_response(
        status_line,
        "application/problem+json",
        &problem_json(status, code),
    )
}

fn capture_client(base_url: &str) -> Client {
    Client::new(base_url, Secret::new("capture-token".into())).unwrap()
}

fn header_value(request: &str, name: &str) -> Option<String> {
    request.lines().skip(1).find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn body_of(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
}

// ── Acceptance: requests match contract paths/headers/bodies ────────────

#[tokio::test]
async fn wire_create_task_sends_contract_path_headers_and_body() {
    let server = capture_server(vec![Some(problem_response("404 Not Found", "not_found"))]).await;
    let client = capture_client(&server.base_url);
    let project = Uuid::now_v7();
    let epic = Uuid::now_v7();
    let key = Uuid::now_v7();

    let result = client
        .create_task(
            project,
            &types::TaskCreate::new(epic, "T".into(), "code".into()),
            &CallOptions::new().idempotency_key(key),
        )
        .await;

    assert_eq!(api_error(result.unwrap_err()).code, "not_found");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let first_line = request.lines().next().unwrap();
    assert_eq!(
        first_line,
        format!("POST /api/v1/projects/{project}/tasks HTTP/1.1")
    );
    assert_eq!(
        header_value(request, "authorization").unwrap(),
        "Bearer capture-token"
    );
    assert_eq!(
        header_value(request, "idempotency-key").unwrap(),
        key.to_string()
    );
    assert_eq!(
        header_value(request, "content-type").unwrap(),
        "application/json"
    );
    let body: serde_json::Value = serde_json::from_str(body_of(request)).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "epic_id": epic, "title": "T", "type_key": "code" })
    );
}

#[tokio::test]
async fn wire_update_task_sends_quoted_if_match_revision() {
    let server = capture_server(vec![Some(problem_response("404 Not Found", "not_found"))]).await;
    let client = capture_client(&server.base_url);
    let patch = types::TaskPatch {
        title: Some("T2".into()),
        ..Default::default()
    };

    let _ = client
        .update_task(Uuid::now_v7(), Uuid::now_v7(), &patch, &keyed().if_match(7))
        .await;

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("PATCH "));
    assert_eq!(header_value(&requests[0], "if-match").unwrap(), "\"7\"");
}

#[tokio::test]
async fn wire_report_claim_sends_lease_token_header() {
    let server = capture_server(vec![Some(problem_response("404 Not Found", "not_found"))]).await;
    let client = capture_client(&server.base_url);

    let _ = client
        .report_claim(
            Uuid::now_v7(),
            Uuid::now_v7(),
            &failed_report(),
            &keyed().lease_token(Secret::new("lease-secret".into())),
        )
        .await;

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        header_value(&requests[0], "x-lease-token").unwrap(),
        "lease-secret"
    );
}

#[tokio::test]
async fn live_full_claim_flow_round_trips_typed_models() {
    let server = live_server().await;
    let client = server.client();
    let h = seed_hierarchy(&client).await;

    let tasks = client
        .list_tasks(h.project.id, &ListTasksQuery::default())
        .await
        .unwrap();
    assert_eq!(tasks.items.len(), 1);
    assert_eq!(tasks.items[0].id, h.task.id);

    let work = client
        .list_work(
            h.project.id,
            types::ClaimInputPhase::Execute,
            &Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(work.items.len(), 1, "unplanned task is executable work");

    let context = client
        .get_task_context(h.project.id, h.task.id)
        .await
        .unwrap();
    assert_eq!(context.epic.id, h.epic.id);

    let grant = claim_execute(&client, &h).await;
    assert!(!grant.lease_token.is_empty());
    assert_eq!(grant.claim.status, types::ClaimStatus::Active);
    assert_eq!(grant.context.project.id, h.project.id);

    let claims = client
        .list_claims(
            h.project.id,
            &ListClaimsQuery {
                task_id: Some(h.task.id),
                active: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(claims.items.len(), 1);

    let lease = Secret::new(grant.lease_token.clone());
    let renewed = client
        .renew_claim(
            h.project.id,
            grant.claim.id,
            &types::RenewInput {
                ttl_seconds: Some(900),
            },
            &keyed().lease_token(lease.clone()),
        )
        .await
        .unwrap();
    assert!(renewed.expires_at > grant.claim.expires_at);

    let session = client
        .report_claim(
            h.project.id,
            grant.claim.id,
            &failed_report(),
            &keyed().lease_token(lease),
        )
        .await
        .unwrap();
    assert_eq!(session.task_id, h.task.id);
    assert_eq!(session.outcome, types::SessionOutcome::Failed);
}

#[tokio::test]
async fn live_get_principal_returns_actor_for_startup_check() {
    let server = live_server().await;
    let owner = server.client();
    let principal = owner.get_principal().await.unwrap();
    assert_eq!(principal.kind, types::ActorKind::Human);

    let issued = owner
        .create_agent(&types::AgentCreate {
            label: "worker".into(),
        })
        .await
        .unwrap();
    let agent = Client::new(&server.base_url, Secret::new(issued.token)).unwrap();
    let principal = agent.get_principal().await.unwrap();
    assert_eq!(principal.kind, types::ActorKind::Agent);
    assert_eq!(principal.id, issued.actor.id);
}

// ── Acceptance: token never appears in Debug output ─────────────────────

#[tokio::test]
async fn debug_output_never_contains_token() {
    let server = live_server().await;
    let token = load_token_file(&server.token_path).unwrap();
    let raw_token = std::fs::read_to_string(&server.token_path)
        .unwrap()
        .trim_end()
        .to_string();
    assert!(!raw_token.is_empty());

    let client = Client::new(&server.base_url, token).unwrap();
    let client_debug = format!("{client:?}");
    assert!(!client_debug.contains(&raw_token), "{client_debug}");

    let opts_debug = format!("{:?}", keyed().lease_token(Secret::new(raw_token.clone())));
    assert!(!opts_debug.contains(&raw_token), "{opts_debug}");

    let wrong = Client::new(&server.base_url, Secret::new(format!("{raw_token}x"))).unwrap();
    let err = wrong.list_projects(&Default::default()).await.unwrap_err();
    let err_debug = format!("{err:?}");
    assert!(!err_debug.contains(&raw_token), "{err_debug}");
    assert_eq!(api_error(err).status, 401);
}

// ── Acceptance: stale revision is not automatically overwritten ─────────

#[tokio::test]
async fn stale_if_match_surfaces_revision_conflict_and_leaves_task_unchanged() {
    let server = live_server().await;
    let client = server.client();
    let h = seed_hierarchy(&client).await;
    let initial_revision = h.task.revision as u64;

    let updated = client
        .update_task(
            h.project.id,
            h.task.id,
            &types::TaskPatch {
                title: Some("T2".into()),
                ..Default::default()
            },
            &keyed().if_match(initial_revision),
        )
        .await
        .unwrap();
    assert!(updated.revision > h.task.revision);

    let err = client
        .update_task(
            h.project.id,
            h.task.id,
            &types::TaskPatch {
                title: Some("T3".into()),
                ..Default::default()
            },
            &keyed().if_match(initial_revision),
        )
        .await
        .unwrap_err();
    let problem = api_error(err);
    assert_eq!(problem.status, 412);
    assert_eq!(problem.code, "revision_conflict");

    let current = client.get_task(h.project.id, h.task.id).await.unwrap();
    assert_eq!(current.title, "T2");
    assert_eq!(current.revision, updated.revision);
}

// ── Acceptance: same-key report retry remains one result ────────────────

#[tokio::test]
async fn same_key_report_claim_replay_yields_single_session() {
    let server = live_server().await;
    let client = server.client();
    let h = seed_hierarchy(&client).await;
    let grant = claim_execute(&client, &h).await;
    let opts = keyed().lease_token(Secret::new(grant.lease_token.clone()));

    let first = client
        .report_claim(h.project.id, grant.claim.id, &failed_report(), &opts)
        .await
        .unwrap();
    let replayed = client
        .report_claim(h.project.id, grant.claim.id, &failed_report(), &opts)
        .await
        .unwrap();
    assert_eq!(first.id, replayed.id);

    let sessions = client
        .list_task_sessions(h.project.id, h.task.id, &PageQuery::default())
        .await
        .unwrap();
    assert_eq!(sessions.items.len(), 1);
}

// ── Acceptance: generated models parse contract payloads ────────────────

#[test]
fn generated_models_deserialize_contract_fixtures() {
    let problem: types::Problem =
        serde_json::from_str(&problem_json(412, "revision_conflict")).unwrap();
    assert_eq!(problem.status, 412);
    assert_eq!(problem.code, "revision_conflict");
    assert_eq!(problem.r#type, "urn:shepherd:error:revision_conflict");

    let event: types::Event = serde_json::from_value(serde_json::json!({
        "id": "42",
        "type": "task.changed",
        "action": "updated",
        "actor_id": "00000000-0000-0000-0000-000000000001",
        "command_id": "00000000-0000-0000-0000-000000000002",
        "project_id": "00000000-0000-0000-0000-000000000003",
        "resource_id": "00000000-0000-0000-0000-000000000004",
        "resource_revision": 2,
        "affected_ids": [],
        "reason": "",
        "occurred_at": "2026-09-14T00:00:00Z",
    }))
    .unwrap();
    assert_eq!(event.id, "42");
    assert_eq!(event.r#type, types::EventType::TaskChanged);
}

// ── Negative scenarios: local transport/credential boundaries ───────────

#[test]
fn non_loopback_url_rejected_as_client_usage() {
    for url in [
        "http://example.com:7437",
        "http://10.0.0.5:7437",
        "https://127.0.0.1:7437",
        "http://user:pw@127.0.0.1:7437",
        "http://127.0.0.1:7437/base",
    ] {
        let err = Client::new(url, Secret::new("t".into())).unwrap_err();
        assert_eq!(err.code(), "client_usage", "{url}");
        assert!(!err.unknown_outcome());
    }
    assert!(Client::new("http://127.0.0.1:7437", Secret::new("t".into())).is_ok());
    assert!(Client::new("http://localhost:7437", Secret::new("t".into())).is_ok());
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = capture_server(vec![Some(
        "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/steal\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into(),
    )])
    .await;
    let client = capture_client(&server.base_url);

    let err = client.get_health().await.unwrap_err();
    assert_eq!(err.status(), Some(302));
    assert_eq!(server.accepted.load(Ordering::SeqCst), 1, "no second fetch");
}

#[cfg(unix)]
#[tokio::test]
async fn group_readable_token_file_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent-token");
    std::fs::write(&path, "secret-token").unwrap();

    for mode in [0o640, 0o604, 0o644] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let err = load_token_file(&path).unwrap_err();
        assert_eq!(err.code(), "client_usage", "mode {mode:o}");
    }

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(load_token_file(&path).is_ok());
}

// ── Negative scenarios: precondition headers are never invented ─────────

#[tokio::test]
async fn missing_idempotency_key_surfaces_428_precondition_required() {
    let server = live_server().await;
    let client = server.client();

    let err = client
        .create_project(&types::ProjectCreate::new("P".into()), &CallOptions::new())
        .await
        .unwrap_err();
    let problem = api_error(err);
    assert_eq!(problem.status, 428);
    assert_eq!(problem.code, "precondition_required");
    assert_eq!(
        client
            .list_projects(&Default::default())
            .await
            .unwrap()
            .items
            .len(),
        0,
        "rejected command persisted nothing"
    );
}

#[tokio::test]
async fn missing_if_match_surfaces_428() {
    let server = live_server().await;
    let client = server.client();
    let h = seed_hierarchy(&client).await;

    let err = client
        .update_task(
            h.project.id,
            h.task.id,
            &types::TaskPatch {
                title: Some("T2".into()),
                ..Default::default()
            },
            &keyed(),
        )
        .await
        .unwrap_err();
    let problem = api_error(err);
    assert_eq!(problem.status, 428);
    assert_eq!(problem.code, "precondition_required");

    let current = client.get_task(h.project.id, h.task.id).await.unwrap();
    assert_eq!(current.title, "T");
    assert_eq!(current.revision, h.task.revision);
}

// ── Negative scenarios: bounded retry with the same key ─────────────────

#[tokio::test]
async fn transport_failure_retries_reads_four_times_then_unknown_outcome() {
    let server = capture_server(vec![None, None, None, None]).await;
    let client = capture_client(&server.base_url);

    let err = client.get_health().await.unwrap_err();
    assert!(err.unknown_outcome());
    assert_eq!(err.code(), "client_transport");
    match err {
        Error::Transport { attempts, .. } => assert_eq!(attempts, 4),
        other => panic!("expected Transport, got {other:?}"),
    }
    assert_eq!(server.accepted.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn mutation_without_key_is_not_retried_on_transport_failure() {
    let server = capture_server(vec![None, None]).await;
    let client = capture_client(&server.base_url);

    let err = client
        .create_project(&types::ProjectCreate::new("P".into()), &CallOptions::new())
        .await
        .unwrap_err();
    assert!(err.unknown_outcome());
    match err {
        Error::Transport { attempts, .. } => assert_eq!(attempts, 1),
        other => panic!("expected Transport, got {other:?}"),
    }
    assert_eq!(server.accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn storage_busy_retries_with_same_key_then_succeeds() {
    let busy = problem_response("503 Service Unavailable", "storage_busy");
    let ok = http_response("200 OK", "application/json", r#"{"ok":true}"#);
    let server = capture_server(vec![Some(busy.clone()), Some(busy), Some(ok)]).await;
    let client = capture_client(&server.base_url);
    let key = Uuid::now_v7();

    let ack = client
        .delete_dependency(
            Uuid::now_v7(),
            Uuid::now_v7(),
            &CallOptions::new().idempotency_key(key).if_match(1),
        )
        .await
        .unwrap();
    assert!(ack.ok);

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert_eq!(
            header_value(request, "idempotency-key").unwrap(),
            key.to_string(),
            "same key on every attempt"
        );
        assert_eq!(header_value(request, "if-match").unwrap(), "\"1\"");
    }
}

#[tokio::test]
async fn renew_without_lease_rejected_and_with_wrong_lease_lease_invalid() {
    let server = live_server().await;
    let client = server.client();
    let h = seed_hierarchy(&client).await;
    let grant = claim_execute(&client, &h).await;
    let renew = types::RenewInput::default();

    let err = client
        .renew_claim(h.project.id, grant.claim.id, &renew, &keyed())
        .await
        .unwrap_err();
    assert_eq!(api_error(err).code, "precondition_required");

    let err = client
        .renew_claim(
            h.project.id,
            grant.claim.id,
            &renew,
            &keyed().lease_token(Secret::new("not-the-lease".into())),
        )
        .await
        .unwrap_err();
    let problem = api_error(err);
    assert_eq!(problem.status, 409);
    assert_eq!(problem.code, "lease_invalid");

    client
        .renew_claim(
            h.project.id,
            grant.claim.id,
            &renew,
            &keyed().lease_token(Secret::new(grant.lease_token.clone())),
        )
        .await
        .unwrap();
}

// ── SSE: resumable event stream ─────────────────────────────────────────

#[tokio::test]
async fn events_stream_yields_typed_change_events_and_tracks_cursor() {
    let server = live_server().await;
    let client = server.client();
    let project = client
        .create_project(&types::ProjectCreate::new("P".into()), &keyed())
        .await
        .unwrap();

    let mut stream = client.get_events(project.id, None).await.unwrap();
    assert_eq!(stream.last_event_id(), None);

    let goal = client
        .create_goal(project.id, &types::GoalCreate::new("G".into()), &keyed())
        .await
        .unwrap();

    match stream.next().await.unwrap().unwrap() {
        EventStreamItem::Change(event) => {
            assert_eq!(event.project_id, project.id);
            assert_eq!(event.resource_id, goal.id);
            assert_eq!(event.r#type, types::EventType::GoalChanged);
        }
        other => panic!("expected change event, got {other:?}"),
    }
    assert!(stream.last_event_id().is_some());
}

#[tokio::test]
async fn events_resume_sends_last_event_id_and_skips_seen_events() {
    let server = live_server().await;
    let client = server.client();
    let project = client
        .create_project(&types::ProjectCreate::new("P".into()), &keyed())
        .await
        .unwrap();

    let mut stream = client.get_events(project.id, None).await.unwrap();
    let goal = client
        .create_goal(project.id, &types::GoalCreate::new("G".into()), &keyed())
        .await
        .unwrap();
    let EventStreamItem::Change(first) = stream.next().await.unwrap().unwrap() else {
        panic!("expected change event");
    };
    assert_eq!(first.resource_id, goal.id);
    let seen = stream.last_event_id().unwrap().to_string();

    // Produced while we are about to reconnect; replay must start after `seen`.
    let epic = client
        .create_epic(
            project.id,
            &types::EpicCreate::new(goal.id, "E".into()),
            &keyed(),
        )
        .await
        .unwrap();

    stream.resume().await.unwrap();
    let EventStreamItem::Change(replayed) = stream.next().await.unwrap().unwrap() else {
        panic!("expected change event");
    };
    assert_eq!(replayed.resource_id, epic.id);
    assert!(
        replayed.id.parse::<u64>().unwrap() > seen.parse::<u64>().unwrap(),
        "replay starts strictly after the last seen event"
    );
}

#[tokio::test]
async fn events_out_of_range_cursor_yields_resync_required_then_close() {
    let server = live_server().await;
    let client = server.client();
    let project = client
        .create_project(&types::ProjectCreate::new("P".into()), &keyed())
        .await
        .unwrap();

    let mut stream = client
        .get_events(project.id, Some("999999".into()))
        .await
        .unwrap();
    match stream.next().await.unwrap().unwrap() {
        EventStreamItem::ResyncRequired { cursor, reason } => {
            assert!(cursor.is_some());
            assert!(!reason.is_empty());
        }
        other => panic!("expected resync_required, got {other:?}"),
    }
    assert!(stream.next().await.is_none(), "stream closes after resync");
}

#[tokio::test]
async fn events_invalid_cursor_rejected_422() {
    let server = live_server().await;
    let client = server.client();
    let project = client
        .create_project(&types::ProjectCreate::new("P".into()), &keyed())
        .await
        .unwrap();

    for cursor in ["abc", "99999999999999999999"] {
        let err = client
            .get_events(project.id, Some(cursor.into()))
            .await
            .unwrap_err();
        let problem = api_error(err);
        assert_eq!(problem.status, 422, "{cursor}");
        assert_eq!(problem.code, "invalid_cursor", "{cursor}");
    }
}
