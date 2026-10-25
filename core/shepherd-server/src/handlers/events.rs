use shepherd_core::model::ProjectId;

use crate::AppState;
use crate::convert::errors::{domain_problem, parse_path_uuid, require_principal, require_store};
use crate::convert::{list_params, page, work};
use crate::generated::server::api::HistoryApi;
use crate::generated::server::errors::ListHistoryResponse;
use crate::generated::types as wire;

#[async_trait::async_trait]
impl HistoryApi for AppState {
    async fn list_history(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        resource_id: Option<String>,
    ) -> ListHistoryResponse {
        let store = require_store!(self, ListHistoryResponse);
        require_principal!(ListHistoryResponse);
        let project = parse_path_uuid!(ListHistoryResponse, project_id, "project_id");
        let resource_id = match resource_id {
            Some(value) => Some(parse_path_uuid!(ListHistoryResponse, value, "resource_id")),
            None => None,
        };
        match store
            .list_history(
                &ProjectId::from_uuid(project),
                resource_id,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, work::event);
                ListHistoryResponse::Ok(wire::PageEvent { items, next_cursor })
            }
            Err(err) => domain_problem!(ListHistoryResponse, err),
        }
    }
}

// ── getEvents: the handwritten resumable SSE route (plan/07, plan/08) ────

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use shepherd_core::model::{Actor, EventId, SecretString};
use shepherd_core::queries::ReplayOutcome;
use shepherd_core::storage::Store;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::convert::request_id;
use crate::middleware::{current_context, problem, problem_body_response, problem_for};

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    after: Option<String>,
}

type Frame = Result<SseEvent, std::convert::Infallible>;

pub(crate) async fn get_events(
    state: AppState,
    Path(project_id): Path<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let request_id = request_id();
    let ctx = current_context();
    let Some(principal) = ctx.0.principal.clone() else {
        // The gate authenticates /api paths; this is unreachable defense.
        return problem_body_response(
            StatusCode::UNAUTHORIZED,
            &problem(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "authentication required",
                request_id,
            ),
        );
    };
    let session_token = ctx.0.session.as_ref().map(|(token, _)| token.clone());
    let store = match state.store_owned() {
        Ok(store) => store,
        Err(err) => {
            let (status, body) = problem_for(&err, request_id);
            return problem_body_response(status, &body);
        }
    };
    let Ok(project) = project_id.parse::<uuid::Uuid>() else {
        return problem_body_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "project_id must be a UUID",
                request_id,
            ),
        );
    };
    let project = ProjectId::from_uuid(project);

    // Last-Event-ID takes precedence over `after`; no cursor starts at the
    // current high-water mark (plan/08).
    let cursor_text = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after);
    let cursor = match cursor_text {
        Some(text) => match parse_cursor(&text) {
            Some(id) => id,
            None => {
                return problem_body_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &problem(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "invalid_cursor",
                        "event cursor must be a decimal event id",
                        request_id,
                    ),
                );
            }
        },
        None => match store.event_high_water().await {
            Ok(id) => id,
            Err(err) => {
                let (status, body) = problem_for(&err, request_id);
                return problem_body_response(status, &body);
            }
        },
    };
    // Route membership before streaming (plan/04).
    if let Err(err) = store.get_project(&project).await {
        let (status, body) = problem_for(&err, request_id);
        return problem_body_response(status, &body);
    }

    // Subscribe before the first replay so no committed event can fall
    // between the snapshot and the live feed (plan/08).
    let notifications = store.subscribe_events();
    let (tx, frames) = mpsc::channel::<Frame>(16);
    tokio::spawn(produce(
        store,
        notifications,
        tx,
        project,
        principal,
        session_token,
        cursor,
    ));
    Sse::new(ReceiverStream::new(frames)).into_response()
}

fn parse_cursor(text: &str) -> Option<EventId> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

async fn produce(
    store: Arc<Store>,
    mut notifications: broadcast::Receiver<EventId>,
    tx: mpsc::Sender<Frame>,
    project: ProjectId,
    principal: Actor,
    session_token: Option<SecretString>,
    mut cursor: EventId,
) {
    if !emit_replay(&store, &tx, &project, &mut cursor).await {
        return;
    }
    // The first tick is also the first liveness recheck, 15s out (plan/08).
    let start = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut heartbeat = tokio::time::interval_at(start, Duration::from_secs(15));
    loop {
        tokio::select! {
            notified = notifications.recv() => match notified {
                Ok(_) => {
                    // Coalesce burst notifications into one replay query.
                    while notifications.try_recv().is_ok() {}
                    if !emit_replay(&store, &tx, &project, &mut cursor).await {
                        return;
                    }
                }
                // Lagging past the channel capacity disconnects the client;
                // retained history recovers it on reconnect (plan/08).
                Err(broadcast::error::RecvError::Lagged(_))
                | Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = heartbeat.tick() => {
                // Comment heartbeat doubles as the revocation/session recheck.
                if tx.send(Ok(SseEvent::default().comment(""))).await.is_err() {
                    return;
                }
                if !caller_live(&store, &principal, &session_token).await {
                    return;
                }
            }
        }
    }
}

/// Emits all persisted events after the cursor; false closes the stream.
async fn emit_replay(
    store: &Store,
    tx: &mpsc::Sender<Frame>,
    project: &ProjectId,
    cursor: &mut EventId,
) -> bool {
    loop {
        match store.replay_events(project, *cursor).await {
            Ok(ReplayOutcome::Events(events)) => {
                if events.is_empty() {
                    return true;
                }
                for event in events {
                    *cursor = event.id;
                    let body = work::event(event);
                    let frame = SseEvent::default()
                        .id(body.id.clone())
                        .event("change")
                        .data(serde_json::to_string(&body).expect("event serializes"));
                    if tx.send(Ok(frame)).await.is_err() {
                        return false;
                    }
                }
            }
            Ok(ReplayOutcome::ResyncRequired {
                current_cursor,
                reason,
            }) => {
                // One explicit resynchronization frame, then close (plan/08).
                let payload = serde_json::json!({
                    "cursor": current_cursor.to_string(),
                    "reason": reason.as_str(),
                });
                let frame = SseEvent::default()
                    .event("resync_required")
                    .data(payload.to_string());
                let _ = tx.send(Ok(frame)).await;
                return false;
            }
            Err(_) => return false,
        }
    }
}

async fn caller_live(
    store: &Store,
    principal: &Actor,
    session_token: &Option<SecretString>,
) -> bool {
    match store.get_actor(&principal.id).await {
        Ok(Some(actor)) if !actor.revoked => {}
        _ => return false,
    }
    if let Some(token) = session_token {
        let now = store.clock().now();
        if store.browser_session(token, now).await.is_err() {
            return false;
        }
    }
    true
}
