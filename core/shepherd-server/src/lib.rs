//! Router and handlers for the shepherd daemon, exposed as a lib target so
//! integration tests (`tests/`) can drive them without booting the binary.
//! Keep this thin: logic belongs in `shepherd-core`.
//!
//! Wire types and routing are generated from the OpenAPI spec by
//! `openapi-to-rust`. The generated code lives in `src/generated/`; this module
//! implements the generated traits by delegating to `shepherd_core::Store`.

mod convert;
// The generated module triggers three clippy style lints by design (collapsed
// `if` chains from the emission template, `must_use` on types already marked
// `must_use`, and `build_router` taking one state per API tag). Everything
// else stays lint-clean.
#[allow(
    clippy::collapsible_if,
    clippy::double_must_use,
    clippy::too_many_arguments
)]
pub mod generated;
mod middleware;

use std::convert::Infallible;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use shepherd_core::Store;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tower_http::services::ServeDir;

use crate::convert::{
    parse_knowledge_id, parse_project_id, parse_relation_id, parse_session_id, parse_task_id,
    problem_detail,
};
use crate::generated::server::api::*;
use crate::generated::server::errors::*;
use crate::generated::types as wire;
use crate::middleware::{ProblemDetailRemapLayer, RateLimitHeaderLayer, cors_layer};

/// Shared application state injected into handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
}

/// The served contract. Embedded so the binary is self-contained.
const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

/// Build the daemon's router.
pub fn router(state: AppState, ui_dir: impl AsRef<Path>) -> Router {
    let generated = generated::server::router::build_router(
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
    );

    let sse_state = state.clone();

    generated
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        .route(
            "/api/v1/events",
            get(move |query: Query<EventsQuery>| sse_handler(sse_state, query)),
        )
        .layer(ProblemDetailRemapLayer)
        .layer(RateLimitHeaderLayer)
        .layer(cors_layer())
        .fallback_service(ServeDir::new(ui_dir.as_ref()))
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}

// ── SSE event stream ──────────────────────────────────────────────────

/// Query parameters for `GET /api/v1/events`.
#[derive(Debug, Deserialize)]
struct EventsQuery {
    project_id: Option<String>,
}

/// SSE handler: streams domain events to the client, optionally filtered
/// by project. No replay — clients refetch on reconnect. A client that
/// lags behind the bus buffer is disconnected (it then reconnects and
/// refetches) rather than silently missing events.
async fn sse_handler(state: AppState, Query(query): Query<EventsQuery>) -> Response {
    // The spec declares a UUID pattern and a 422 for a malformed filter —
    // do not silently stream nothing for a typo'd id.
    let project_filter = match query.project_id {
        Some(pid) => match parse_project_id(&pid) {
            Ok(id) => Some(id),
            Err(e) => return validation_problem_detail(&e),
        },
        None => None,
    };

    let rx = state.store.subscribe();

    // On lag the stream ends: the client disconnects, reconnects, and
    // refetches — the documented recovery path. Silently skipping the
    // lost events would leave the client's view stale with no way to
    // notice.
    let stream = BroadcastStream::new(rx)
        .take_while(|result| match result {
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                println!(
                    "sse: client lagged ({n} events lost), disconnecting — \
                     client refetches on reconnect"
                );
                false
            }
            _ => true,
        })
        .filter_map(move |result| match result {
            Ok(event) => {
                // Apply project filter.
                if let Some(pid) = project_filter
                    && event.project_id() != pid
                {
                    return None;
                }

                let sse_event = Event::default()
                    .event(event.event_type())
                    .json_data(event.to_payload())
                    .ok()?;

                Some(Ok::<Event, Infallible>(sse_event))
            }
            // Unreachable: take_while ends the stream on Lagged.
            Err(_) => None,
        });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

/// 422 problem-detail response, matching the generated error responses.
fn validation_problem_detail(e: &shepherd_core::Error) -> Response {
    let mut response = (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        axum::Json(problem_detail(e)),
    )
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// Spawn the background claim sweeper: every `period`, expired leases are
/// released and their tasks returned to `ready`, so a crashed agent's task
/// is freed without waiting for API traffic (the lazy sweep in `next-task`).
/// Returns the task handle; abort it to stop sweeping.
pub fn spawn_claim_sweeper(
    store: Store,
    period: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match store.sweep_expired_claims(chrono::Utc::now()).await {
                Ok(released) => {
                    for task_id in released {
                        println!("sweep: expired lease released task {task_id}");
                    }
                }
                Err(e) => eprintln!("sweep: error releasing expired claims: {e}"),
            }
        }
    })
}

// ── Macro to reduce error-mapping boilerplate ───────────────────────────

/// Map a `shepherd_core::Error` to the matching response enum variant.
/// Response enums with NotFound + Conflict variants (no Gone — 410 cannot
/// arise on these endpoints; folding it into Conflict is defensive only).
macro_rules! map_err {
    ($resp:ident, $err:expr) => {
        match $err.status_code() {
            404 => $resp::NotFound(problem_detail(&$err)),
            409 | 410 => $resp::Conflict(problem_detail(&$err)),
            422 => $resp::UnprocessableEntity(problem_detail(&$err)),
            _ => $resp::InternalServerError(problem_detail(&$err)),
        }
    };
}

/// For response enums with NotFound + Conflict + Gone variants (claim renew/
/// release, session report — the lease endpoints).
macro_rules! map_err_gone {
    ($resp:ident, $err:expr) => {
        match $err.status_code() {
            404 => $resp::NotFound(problem_detail(&$err)),
            409 => $resp::Conflict(problem_detail(&$err)),
            410 => $resp::Gone(problem_detail(&$err)),
            422 => $resp::UnprocessableEntity(problem_detail(&$err)),
            _ => $resp::InternalServerError(problem_detail(&$err)),
        }
    };
}

/// For response enums with NotFound but no Conflict.
macro_rules! map_err_no_conflict {
    ($resp:ident, $err:expr) => {
        match $err.status_code() {
            404 => $resp::NotFound(problem_detail(&$err)),
            422 => $resp::UnprocessableEntity(problem_detail(&$err)),
            _ => $resp::InternalServerError(problem_detail(&$err)),
        }
    };
}

/// For response enums with neither NotFound nor Conflict (list/create at root level).
macro_rules! map_err_minimal {
    ($resp:ident, $err:expr) => {
        match $err.status_code() {
            422 => $resp::UnprocessableEntity(problem_detail(&$err)),
            _ => $resp::InternalServerError(problem_detail(&$err)),
        }
    };
}

// ── SystemApi ───────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl SystemApi for AppState {
    async fn get_health(&self) -> GetHealthResponse {
        GetHealthResponse::Ok(wire::Health {
            status: wire::HealthStatus::Pass,
            version: shepherd_core::version().to_string(),
            description: Some("shepherd local daemon".to_string()),
            output: None,
            notes: None,
        })
    }
}

// ── ProjectsApi ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl ProjectsApi for AppState {
    async fn list_projects(
        &self,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListProjectsResponse {
        let limit = limit.map(|l| l as i64).unwrap_or(25);
        match self.store.list_projects(cursor.as_deref(), limit).await {
            Ok(page) => ListProjectsResponse::Ok(page.into()),
            Err(e) => map_err_minimal!(ListProjectsResponse, e),
        }
    }

    async fn create_project(&self, body: wire::ProjectCreate) -> CreateProjectResponse {
        let input: shepherd_core::ProjectCreate = body.into();
        match self.store.create_project(&input).await {
            Ok(project) => CreateProjectResponse::Created(project.into()),
            Err(e) => map_err_minimal!(CreateProjectResponse, e),
        }
    }

    async fn get_project(&self, project_id: String) -> GetProjectResponse {
        let id = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetProjectResponse, e),
        };
        match self.store.get_project(id).await {
            Ok(project) => GetProjectResponse::Ok(project.into()),
            Err(e) => map_err_no_conflict!(GetProjectResponse, e),
        }
    }

    async fn update_project(
        &self,
        project_id: String,
        body: wire::ProjectUpdate,
    ) -> UpdateProjectResponse {
        let id = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(UpdateProjectResponse, e),
        };
        let input: shepherd_core::ProjectUpdate = body.into();
        match self.store.update_project(id, &input).await {
            Ok(project) => UpdateProjectResponse::Ok(project.into()),
            Err(e) => map_err_no_conflict!(UpdateProjectResponse, e),
        }
    }

    async fn delete_project(&self, project_id: String) -> DeleteProjectResponse {
        let id = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteProjectResponse, e),
        };
        match self.store.delete_project(id).await {
            Ok(()) => DeleteProjectResponse::NoContent,
            Err(e) => map_err_no_conflict!(DeleteProjectResponse, e),
        }
    }
}

// ── TasksApi ────────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl TasksApi for AppState {
    async fn list_tasks(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        status: Option<ListTasksStatus>,
        r#type: Option<ListTasksType>,
    ) -> ListTasksResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListTasksResponse, e),
        };
        let limit = limit.map(|l| l as i64).unwrap_or(25);
        let core_status = status.map(|s| match s {
            ListTasksStatus::Proposed => shepherd_core::TaskStatus::Proposed,
            ListTasksStatus::Approved => shepherd_core::TaskStatus::Approved,
            ListTasksStatus::Ready => shepherd_core::TaskStatus::Ready,
            ListTasksStatus::InProgress => shepherd_core::TaskStatus::InProgress,
            ListTasksStatus::InReview => shepherd_core::TaskStatus::InReview,
            ListTasksStatus::Done => shepherd_core::TaskStatus::Done,
            ListTasksStatus::Blocked => shepherd_core::TaskStatus::Blocked,
            ListTasksStatus::Cancelled => shepherd_core::TaskStatus::Cancelled,
        });
        let core_type = r#type.map(|t| match t {
            ListTasksType::Code => shepherd_core::TaskType::Code,
            ListTasksType::Question => shepherd_core::TaskType::Question,
            ListTasksType::Refactor => shepherd_core::TaskType::Refactor,
            ListTasksType::Review => shepherd_core::TaskType::Review,
            ListTasksType::Research => shepherd_core::TaskType::Research,
        });
        match self
            .store
            .list_tasks(pid, cursor.as_deref(), limit, core_status, core_type)
            .await
        {
            Ok(page) => ListTasksResponse::Ok(page.into()),
            Err(e) => map_err_no_conflict!(ListTasksResponse, e),
        }
    }

    async fn create_task(&self, project_id: String, body: wire::TaskCreate) -> CreateTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(CreateTaskResponse, e),
        };
        let input: shepherd_core::TaskCreate = body.into();
        match self.store.create_task(pid, &input).await {
            Ok(task) => CreateTaskResponse::Created(task.into()),
            Err(e) => map_err_no_conflict!(CreateTaskResponse, e),
        }
    }

    async fn get_task(&self, project_id: String, task_id: String) -> GetTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskResponse, e),
        };
        match self.store.get_task(pid, tid).await {
            Ok(task) => GetTaskResponse::Ok(task.into()),
            Err(e) => map_err_no_conflict!(GetTaskResponse, e),
        }
    }

    async fn update_task(
        &self,
        project_id: String,
        task_id: String,
        body: wire::TaskUpdate,
    ) -> UpdateTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(UpdateTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(UpdateTaskResponse, e),
        };
        let input: shepherd_core::TaskUpdate = body.into();
        match self.store.update_task(pid, tid, &input).await {
            Ok(task) => UpdateTaskResponse::Ok(task.into()),
            Err(e) => map_err_no_conflict!(UpdateTaskResponse, e),
        }
    }

    async fn delete_task(&self, project_id: String, task_id: String) -> DeleteTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteTaskResponse, e),
        };
        match self.store.delete_task(pid, tid).await {
            Ok(()) => DeleteTaskResponse::NoContent,
            Err(e) => map_err_no_conflict!(DeleteTaskResponse, e),
        }
    }

    async fn approve_task(
        &self,
        project_id: String,
        task_id: String,
        _body: Option<wire::ApprovalRequest>,
    ) -> ApproveTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(ApproveTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(ApproveTaskResponse, e),
        };
        match self.store.approve_task(pid, tid).await {
            Ok(task) => ApproveTaskResponse::Ok(task.into()),
            Err(e) => map_err!(ApproveTaskResponse, e),
        }
    }

    async fn reject_task(
        &self,
        project_id: String,
        task_id: String,
        body: wire::RejectionRequest,
    ) -> RejectTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(RejectTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(RejectTaskResponse, e),
        };
        match self.store.reject_task(pid, tid, &body.reason).await {
            Ok(task) => RejectTaskResponse::Ok(task.into()),
            Err(e) => map_err!(RejectTaskResponse, e),
        }
    }

    async fn block_task(
        &self,
        project_id: String,
        task_id: String,
        body: wire::BlockRequest,
    ) -> BlockTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(BlockTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(BlockTaskResponse, e),
        };
        match self.store.block_task(pid, tid, &body.reason).await {
            Ok(task) => BlockTaskResponse::Ok(task.into()),
            Err(e) => map_err!(BlockTaskResponse, e),
        }
    }

    async fn unblock_task(&self, project_id: String, task_id: String) -> UnblockTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(UnblockTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(UnblockTaskResponse, e),
        };
        match self.store.unblock_task(pid, tid).await {
            Ok(task) => UnblockTaskResponse::Ok(task.into()),
            Err(e) => map_err!(UnblockTaskResponse, e),
        }
    }

    async fn cancel_task(&self, project_id: String, task_id: String) -> CancelTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(CancelTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(CancelTaskResponse, e),
        };
        match self.store.cancel_task(pid, tid).await {
            Ok(task) => CancelTaskResponse::Ok(task.into()),
            Err(e) => map_err!(CancelTaskResponse, e),
        }
    }

    async fn get_next_task(&self, project_id: String) -> GetNextTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetNextTaskResponse, e),
        };
        match self.store.next_task(pid).await {
            Ok(task) => GetNextTaskResponse::Ok(wire::NextTaskResult {
                task: task.map(wire::Task::from),
            }),
            Err(e) => map_err_no_conflict!(GetNextTaskResponse, e),
        }
    }

    async fn get_task_context(
        &self,
        project_id: String,
        task_id: String,
    ) -> GetTaskContextResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskContextResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskContextResponse, e),
        };
        match self.store.get_context_bundle(pid, tid).await {
            Ok(bundle) => GetTaskContextResponse::Ok(bundle.into()),
            Err(e) => map_err_no_conflict!(GetTaskContextResponse, e),
        }
    }
}

// ── ClaimsApi ───────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl ClaimsApi for AppState {
    async fn claim_task(
        &self,
        project_id: String,
        task_id: String,
        body: wire::ClaimRequest,
    ) -> ClaimTaskResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(ClaimTaskResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(ClaimTaskResponse, e),
        };
        let input: shepherd_core::ClaimRequest = body.into();
        match self
            .store
            .claim_task(pid, tid, &input, chrono::Utc::now())
            .await
        {
            Ok(claim) => ClaimTaskResponse::Created(claim.into()),
            Err(e) => map_err!(ClaimTaskResponse, e),
        }
    }

    async fn renew_claim(
        &self,
        project_id: String,
        task_id: String,
        body: wire::ClaimRenewal,
    ) -> RenewClaimResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(RenewClaimResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(RenewClaimResponse, e),
        };
        let input: shepherd_core::ClaimRenewal = body.into();
        match self
            .store
            .renew_claim(pid, tid, &input, chrono::Utc::now())
            .await
        {
            Ok(claim) => RenewClaimResponse::Ok(claim.into()),
            Err(e) => map_err_gone!(RenewClaimResponse, e),
        }
    }

    async fn release_claim(
        &self,
        project_id: String,
        task_id: String,
        body: wire::ClaimRelease,
    ) -> ReleaseClaimResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(ReleaseClaimResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(ReleaseClaimResponse, e),
        };
        let input: shepherd_core::ClaimRelease = body.into();
        match self
            .store
            .release_claim(pid, tid, &input, chrono::Utc::now())
            .await
        {
            Ok(()) => ReleaseClaimResponse::NoContent,
            Err(e) => map_err_gone!(ReleaseClaimResponse, e),
        }
    }
}

// ── SessionsApi ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl SessionsApi for AppState {
    async fn list_task_sessions(
        &self,
        project_id: String,
        task_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListTaskSessionsResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListTaskSessionsResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListTaskSessionsResponse, e),
        };
        let limit = limit.map(|l| l as i64).unwrap_or(25);
        match self
            .store
            .list_sessions(pid, tid, cursor.as_deref(), limit)
            .await
        {
            Ok(page) => ListTaskSessionsResponse::Ok(page.into()),
            Err(e) => map_err_no_conflict!(ListTaskSessionsResponse, e),
        }
    }

    async fn create_task_session(
        &self,
        project_id: String,
        task_id: String,
        body: wire::SessionReport,
    ) -> CreateTaskSessionResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(CreateTaskSessionResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_gone!(CreateTaskSessionResponse, e),
        };
        let input: shepherd_core::SessionReport = body.into();
        match self
            .store
            .create_session(pid, tid, &input, chrono::Utc::now())
            .await
        {
            Ok(session) => CreateTaskSessionResponse::Created(session.into()),
            Err(e) => map_err_gone!(CreateTaskSessionResponse, e),
        }
    }

    async fn get_task_session(
        &self,
        project_id: String,
        task_id: String,
        session_id: String,
    ) -> GetTaskSessionResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskSessionResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskSessionResponse, e),
        };
        let sid = match parse_session_id(&session_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetTaskSessionResponse, e),
        };
        match self.store.get_session(pid, tid, sid).await {
            Ok(session) => GetTaskSessionResponse::Ok(session.into()),
            Err(e) => map_err_no_conflict!(GetTaskSessionResponse, e),
        }
    }
}

// ── KnowledgeApi ────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl KnowledgeApi for AppState {
    async fn list_knowledge(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        scope: Option<ListKnowledgeScope>,
        r#type: Option<ListKnowledgeType>,
        task_id: Option<String>,
    ) -> ListKnowledgeResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListKnowledgeResponse, e),
        };
        let tid = match task_id.as_deref().map(parse_task_id).transpose() {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListKnowledgeResponse, e),
        };
        let limit = limit.map(|l| l as i64).unwrap_or(25);
        let core_scope = scope.map(|s| match s {
            ListKnowledgeScope::Task => shepherd_core::KnowledgeScope::Task,
            ListKnowledgeScope::Session => shepherd_core::KnowledgeScope::Session,
            ListKnowledgeScope::Project => shepherd_core::KnowledgeScope::Project,
        });
        let core_type = r#type.map(|t| match t {
            ListKnowledgeType::Link => shepherd_core::KnowledgeType::Link,
            ListKnowledgeType::Transcript => shepherd_core::KnowledgeType::Transcript,
            ListKnowledgeType::Decision => shepherd_core::KnowledgeType::Decision,
            ListKnowledgeType::Note => shepherd_core::KnowledgeType::Note,
        });
        match self
            .store
            .list_knowledge(pid, cursor.as_deref(), limit, core_scope, core_type, tid)
            .await
        {
            Ok(page) => ListKnowledgeResponse::Ok(page.into()),
            Err(e) => map_err_no_conflict!(ListKnowledgeResponse, e),
        }
    }

    async fn create_knowledge(
        &self,
        project_id: String,
        body: wire::KnowledgeItemCreate,
    ) -> CreateKnowledgeResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(CreateKnowledgeResponse, e),
        };
        let input: shepherd_core::KnowledgeItemCreate = body.into();
        match self.store.create_knowledge(pid, &input).await {
            Ok(item) => CreateKnowledgeResponse::Created(item.into()),
            Err(e) => map_err_no_conflict!(CreateKnowledgeResponse, e),
        }
    }

    async fn get_knowledge(
        &self,
        project_id: String,
        knowledge_id: String,
    ) -> GetKnowledgeResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetKnowledgeResponse, e),
        };
        let kid = match parse_knowledge_id(&knowledge_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(GetKnowledgeResponse, e),
        };
        match self.store.get_knowledge(pid, kid).await {
            Ok(item) => GetKnowledgeResponse::Ok(item.into()),
            Err(e) => map_err_no_conflict!(GetKnowledgeResponse, e),
        }
    }

    async fn delete_knowledge(
        &self,
        project_id: String,
        knowledge_id: String,
    ) -> DeleteKnowledgeResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteKnowledgeResponse, e),
        };
        let kid = match parse_knowledge_id(&knowledge_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteKnowledgeResponse, e),
        };
        match self.store.delete_knowledge(pid, kid).await {
            Ok(()) => DeleteKnowledgeResponse::NoContent,
            Err(e) => map_err_no_conflict!(DeleteKnowledgeResponse, e),
        }
    }
}

// ── ExportImportApi ─────────────────────────────────────────────────────

#[async_trait::async_trait]
impl ExportImportApi for AppState {
    async fn export_project(&self, project_id: String) -> ExportProjectResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ExportProjectResponse, e),
        };
        match self.store.export_project(pid).await {
            Ok(doc) => ExportProjectResponse::Ok(doc.into()),
            Err(e) => map_err_no_conflict!(ExportProjectResponse, e),
        }
    }

    async fn import_project(&self, body: wire::ExportDocument) -> ImportProjectResponse {
        let doc: shepherd_core::ExportDocument = body.into();
        match self.store.import_project(&doc).await {
            Ok(result) => ImportProjectResponse::Created(result.into()),
            Err(e) => map_err_minimal!(ImportProjectResponse, e),
        }
    }
}

// ── RelationsApi ────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl RelationsApi for AppState {
    async fn list_project_relations(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListProjectRelationsResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListProjectRelationsResponse, e),
        };
        let limit = limit.map(|l| l as i64).unwrap_or(25);
        match self
            .store
            .list_project_relations(pid, cursor.as_deref(), limit)
            .await
        {
            Ok(page) => ListProjectRelationsResponse::Ok(page.into()),
            Err(e) => map_err_no_conflict!(ListProjectRelationsResponse, e),
        }
    }

    async fn list_task_relations(
        &self,
        project_id: String,
        task_id: String,
    ) -> ListTaskRelationsResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListTaskRelationsResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(ListTaskRelationsResponse, e),
        };
        match self.store.list_relations(pid, tid).await {
            Ok(rels) => ListTaskRelationsResponse::Ok(wire::RelationList {
                items: rels.into_iter().map(wire::Relation::from).collect(),
            }),
            Err(e) => map_err_no_conflict!(ListTaskRelationsResponse, e),
        }
    }

    async fn create_task_relation(
        &self,
        project_id: String,
        task_id: String,
        body: wire::RelationCreate,
    ) -> CreateTaskRelationResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err!(CreateTaskRelationResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err!(CreateTaskRelationResponse, e),
        };
        let input: shepherd_core::RelationCreate = body.into();
        match self.store.create_relation(pid, tid, &input).await {
            Ok(rel) => CreateTaskRelationResponse::Created(rel.into()),
            Err(e) => map_err!(CreateTaskRelationResponse, e),
        }
    }

    async fn delete_task_relation(
        &self,
        project_id: String,
        task_id: String,
        relation_id: String,
    ) -> DeleteTaskRelationResponse {
        let pid = match parse_project_id(&project_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteTaskRelationResponse, e),
        };
        let tid = match parse_task_id(&task_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteTaskRelationResponse, e),
        };
        let rid = match parse_relation_id(&relation_id) {
            Ok(id) => id,
            Err(e) => return map_err_no_conflict!(DeleteTaskRelationResponse, e),
        };
        match self.store.delete_relation(pid, tid, rid).await {
            Ok(()) => DeleteTaskRelationResponse::NoContent,
            Err(e) => map_err_no_conflict!(DeleteTaskRelationResponse, e),
        }
    }
}
