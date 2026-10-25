// Resumable SSE over live HTTP: replay, live delivery, heartbeats, resync
// outcomes and disconnect rules (plan/08). The replay query itself is covered
// by the core history suite.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{World, seed_hierarchy, world};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shepherd_core::storage::{open, testing};
use shepherd_server::{AppState, AuthConfig};
use tower::ServiceExt;

#[derive(Debug, Default, PartialEq)]
struct Frame {
    id: Option<String>,
    event: Option<String>,
    data: String,
    comment: bool,
}

struct EventStream {
    body: Body,
    buffer: String,
    ended: bool,
}

impl EventStream {
    /// Next full SSE frame, or None once the stream has closed.
    async fn next_frame(&mut self) -> Option<Frame> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let raw = self.buffer[..end].to_string();
                self.buffer.drain(..end + 2);
                return Some(parse_frame(&raw));
            }
            if self.ended {
                return None;
            }
            match self.body.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(bytes) = frame.into_data() {
                        self.buffer.push_str(std::str::from_utf8(&bytes).unwrap());
                    }
                }
                Some(Err(_)) | None => self.ended = true,
            }
        }
    }

    async fn next_change(&mut self) -> Option<Frame> {
        while let Some(frame) = self.next_frame().await {
            if !frame.comment {
                return Some(frame);
            }
        }
        None
    }
}

fn parse_frame(raw: &str) -> Frame {
    let mut frame = Frame::default();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            frame.data.push_str(rest.trim_start());
        } else if let Some(rest) = line.strip_prefix("id:") {
            frame.id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("event:") {
            frame.event = Some(rest.trim().to_string());
        } else if line.starts_with(':') {
            frame.comment = true;
        }
    }
    frame
}

async fn connect(w: &World, project: &str, extra: &[(&str, &str)]) -> EventStream {
    connect_app(&w.app, &w.owner_token, project, extra).await
}

async fn connect_app(
    app: &axum::Router,
    token: &str,
    project: &str,
    extra: &[(&str, &str)],
) -> EventStream {
    let mut request = Request::get(format!("/api/v1/projects/{project}/events"))
        .header(header::HOST, common::HOST)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    EventStream {
        body: response.into_body(),
        buffer: String::new(),
        ended: false,
    }
}

async fn create_goal(w: &World, project: &str, title: &str) -> Value {
    let (status, _, body) = w
        .owner(Method::POST, &format!("/api/v1/projects/{project}/goals"))
        .idempotency()
        .json(json!({ "title": title }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn history_high_water(w: &World, project: &str) -> i64 {
    let (_, _, history) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{project}/history?limit=1"),
        )
        .send(&w.app)
        .await;
    history["items"][0]["id"].as_str().unwrap().parse().unwrap()
}

async fn read(stream: &mut EventStream) -> Option<Frame> {
    tokio::time::timeout(Duration::from_secs(5), stream.next_change())
        .await
        .expect("frame within timeout")
}

#[tokio::test]
async fn sse_streams_committed_events_with_decimal_ids_and_change_frames() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    // Replay from 0 covers the already-committed history in id order.
    let mut stream = connect(&w, &h.project, &[("last-event-id", "0")]).await;
    let first = read(&mut stream).await.unwrap();
    assert_eq!(first.event.as_deref(), Some("change"));
    assert_eq!(first.id.as_deref(), Some("1"));
    let body: Value = serde_json::from_str(&first.data).unwrap();
    assert_eq!(body["id"], "1");
    assert_eq!(body["project_id"], json!(h.project));

    // Drain the replay up to the current high-water mark, ids ascending.
    let high_water = history_high_water(&w, &h.project).await;
    let mut last_id: i64 = 1;
    while last_id < high_water {
        let frame = read(&mut stream).await.unwrap();
        let id: i64 = frame.id.as_deref().unwrap().parse().unwrap();
        assert!(id > last_id, "ids ascend");
        last_id = id;
    }

    // A live commit arrives as the next frame.
    let goal = create_goal(&w, &h.project, "Live goal").await;
    let frame = read(&mut stream).await.unwrap();
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["type"], "goal.changed");
    assert_eq!(body["resource_id"], goal["id"]);
    assert!(frame.id.as_deref().unwrap().parse::<i64>().unwrap() > last_id);
}

#[tokio::test]
async fn sse_last_event_id_takes_precedence_over_after() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let goal = create_goal(&w, &h.project, "Marker").await;

    // after=0 would replay everything; Last-Event-ID at the high-water mark
    // wins, so only events after the marker arrive.
    let high_water = history_high_water(&w, &h.project).await.to_string();
    let mut stream = connect(
        &w,
        &h.project,
        &[("last-event-id", high_water.as_str()), ("after", "0")],
    )
    .await;
    let _ = goal;
    create_goal(&w, &h.project, "After the marker").await;
    let frame = read(&mut stream).await.unwrap();
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["reason"], "");
    assert_eq!(body["action"], "createGoal");
    assert!(
        frame.id.unwrap().parse::<i64>().unwrap() > high_water.parse::<i64>().unwrap(),
        "replay starts strictly after Last-Event-ID"
    );
}

#[tokio::test]
async fn sse_resumes_after_lost_connection_without_gaps() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let mut stream = connect(&w, &h.project, &[]).await;
    create_goal(&w, &h.project, "First").await;
    let frame = read(&mut stream).await.unwrap();
    let last_seen = frame.id.unwrap();
    drop(stream);

    // Events committed while disconnected replay on reconnect, gap-free.
    create_goal(&w, &h.project, "While away").await;
    let mut stream = connect(&w, &h.project, &[("last-event-id", last_seen.as_str())]).await;
    let frame = read(&mut stream).await.unwrap();
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["action"], "createGoal");
    assert_eq!(
        frame.id.unwrap().parse::<i64>().unwrap(),
        last_seen.parse::<i64>().unwrap() + 1,
        "no gap between the last seen and the first replayed event"
    );
}

#[tokio::test]
async fn sse_restart_resumes_ids_from_database() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let mut stream = connect(&w, &h.project, &[]).await;
    create_goal(&w, &h.project, "Before restart").await;
    let before = read(&mut stream).await.unwrap().id.unwrap();
    drop(stream);

    // A second store over the same database resumes the id sequence.
    let store = Arc::new(
        open(testing::store_options(
            w._dir.path(),
            "shepherd.db",
            w.clock.clone(),
        ))
        .await
        .unwrap(),
    );
    let app = shepherd_server::router(
        AppState::new(store, AuthConfig::new(7437, false)),
        "does-not-exist",
    );
    let mut stream = connect_app(
        &app,
        &w.owner_token,
        &h.project,
        &[("last-event-id", before.as_str())],
    )
    .await;
    let request = Request::post(format!("/api/v1/projects/{}/goals", h.project))
        .header(header::HOST, common::HOST)
        .header(header::AUTHORIZATION, format!("Bearer {}", w.owner_token))
        .header(
            "idempotency-key",
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "title": "After restart" }).to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let frame = read(&mut stream).await.unwrap();
    assert_eq!(
        frame.id.unwrap().parse::<i64>().unwrap(),
        before.parse::<i64>().unwrap() + 1
    );
}

#[tokio::test]
async fn sse_cursor_ahead_cursor_expired_and_backlog_limit_emit_resync_and_close() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;

    // Cursor ahead of the database (post-restore state).
    let mut stream = connect(&w, &h.project, &[("last-event-id", "999999")]).await;
    let frame = read(&mut stream).await.unwrap();
    assert_eq!(frame.event.as_deref(), Some("resync_required"));
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["reason"], "cursor_ahead");
    assert!(body["cursor"].as_str().unwrap().parse::<i64>().is_ok());
    assert!(stream.next_change().await.is_none(), "stream closes");

    // A cursor older than the seven-day replay window.
    w.clock.advance(chrono::TimeDelta::days(8));
    let mut stream = connect(&w, &h.project, &[("last-event-id", "0")]).await;
    let frame = read(&mut stream).await.unwrap();
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["reason"], "cursor_expired");
    assert!(stream.next_change().await.is_none(), "stream closes");

    // A backlog past the replay bound, seeded fresh so the window check
    // passes and the bound is what triggers.
    let pre_seed_high_water = history_high_water(&w, &h.project).await;
    let project = h.project.clone();
    let actor = {
        let (_, _, principal) = w.owner(Method::GET, "/api/v1/principal").send(&w.app).await;
        principal["id"].as_str().unwrap().to_string()
    };
    let occurred = "2026-09-22T00:00:00.000Z";
    w.store
        .command_transaction(move |tx| {
            Box::pin(async move {
                for _ in 0..1_001 {
                    sqlx::query(
                        "INSERT INTO events (action, reason, project_id, actor_id, command_id, \
                         type, resource_id, resource_revision, occurred_at, affected_ids) \
                         VALUES ('seed', '', ?1, ?2, ?3, 'task.changed', ?4, 1, ?5, '[]')",
                    )
                    .bind(&project)
                    .bind(&actor)
                    .bind(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
                    .bind(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
                    .bind(occurred)
                    .execute(&mut **tx)
                    .await?;
                }
                Ok(())
            })
        })
        .await
        .unwrap();
    let mut stream = connect(
        &w,
        &h.project,
        &[("last-event-id", &pre_seed_high_water.to_string())],
    )
    .await;
    let frame = read(&mut stream).await.unwrap();
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    assert_eq!(body["reason"], "backlog_limit");
    assert!(stream.next_change().await.is_none(), "stream closes");
}

#[tokio::test]
async fn sse_sends_comment_heartbeats_without_ids() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let mut stream = connect(&w, &h.project, &[]).await;
    // Pause after setup (SQLite open uses real timers); auto-advance then
    // fires the 15s heartbeat interval instantly while the test is idle.
    tokio::time::pause();
    let frame = stream.next_frame().await.unwrap();
    assert!(frame.comment, "{frame:?}");
    assert!(frame.id.is_none());
    assert!(frame.event.is_none());
    let frame = stream.next_frame().await.unwrap();
    assert!(frame.comment, "{frame:?}");
}

#[tokio::test]
async fn sse_revoked_actor_closes_on_heartbeat_recheck() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let agent = w.agent_token("watcher").await;
    let (_, _, agents) = w.owner(Method::GET, "/api/v1/agents").send(&w.app).await;
    let agent_id = agents["items"][0]["id"].as_str().unwrap().to_string();

    let mut stream = connect_app(&w.app, &agent, &h.project, &[]).await;
    let (status, _, body) = w
        .owner(Method::POST, &format!("/api/v1/agents/{agent_id}/revoke"))
        .idempotency()
        .json(json!({ "reason": "stream must drop" }))
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The next heartbeat recheck notices the revocation and closes.
    tokio::time::pause();
    let mut frames = 0;
    while stream.next_frame().await.is_some() {
        frames += 1;
        assert!(frames < 50, "stream must close after the recheck");
    }
}

#[tokio::test]
async fn sse_lagging_client_disconnects_after_256_notifications() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;
    let mut stream = connect(&w, &h.project, &[]).await;

    // An unread client back-pressures the producer; 300 further commits
    // overflow its 256-notification budget and the stream must close.
    for index in 0..300 {
        create_goal(&w, &h.project, &format!("burst {index}")).await;
    }
    let mut frames = 0;
    while tokio::time::timeout(Duration::from_secs(10), stream.next_frame())
        .await
        .expect("stream makes progress")
        .is_some()
    {
        frames += 1;
        assert!(frames < 2_000, "stream must close after lagging");
    }
}

#[tokio::test]
async fn sse_requires_auth_and_404s_unknown_project() {
    let w = world().await;
    let h = seed_hierarchy(&w).await;

    let response = w
        .req(
            Method::GET,
            &format!("/api/v1/projects/{}/events", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);

    let (status, _, body) = w
        .owner(
            Method::GET,
            "/api/v1/projects/00000000-0000-7000-8000-000000000000/events",
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "not_found");

    let (status, _, body) = w
        .owner(
            Method::GET,
            &format!("/api/v1/projects/{}/events?after=junk", h.project),
        )
        .send(&w.app)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "invalid_cursor");
}

#[tokio::test]
async fn sse_frames_validate_against_the_embedded_asyncapi_contract() {
    let asyncapi: Value = serde_json::from_str(include_str!(
        "../../../openapi/shepherd-events.asyncapi.yaml"
    ))
    .expect("asyncapi contract parses");
    let w = world().await;
    let h = seed_hierarchy(&w).await;

    let mut stream = connect(&w, &h.project, &[]).await;
    create_goal(&w, &h.project, "Validated").await;
    let frame = read(&mut stream).await.unwrap();
    assert_eq!(frame.event.as_deref(), Some("change"));
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    let change_schema = &asyncapi["components"]["messages"]["Change"]["payload"];
    common::validate_schema_value(&asyncapi, change_schema, &body, "Change payload");

    drop(stream);
    let mut stream = connect(&w, &h.project, &[("last-event-id", "999999")]).await;
    let frame = read(&mut stream).await.unwrap();
    assert_eq!(frame.event.as_deref(), Some("resync_required"));
    let body: Value = serde_json::from_str(&frame.data).unwrap();
    let resync_schema = &asyncapi["components"]["messages"]["Resync"]["payload"];
    common::validate_schema_value(&asyncapi, resync_schema, &body, "Resync payload");
}
