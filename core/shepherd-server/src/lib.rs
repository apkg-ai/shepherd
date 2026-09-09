//! Router and handlers for the shepherd daemon, exposed as a lib target so
//! integration tests (`tests/`) can drive them without booting the binary.
//! Keep this thin: logic belongs in `shepherd-core`.
//!
//! Wire types and routing are generated from the OpenAPI spec by
//! `openapi-to-rust`. The generated code lives in `src/generated/`; this module
//! implements the generated traits by delegating to `shepherd_core::Store`.

mod convert;
// The generated module triggers two clippy style lints by design (collapsed
// `if` chains from the emission template, `must_use` on types already marked
// `must_use`). Everything else stays lint-clean.
#[allow(clippy::collapsible_if, clippy::double_must_use)]
pub mod generated;
mod middleware;

use std::path::Path;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use shepherd_core::Store;
use tower_http::services::ServeDir;

use crate::convert::{parse_project_id, parse_relation_id, parse_task_id, problem_detail};
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
    );

    generated
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        .layer(ProblemDetailRemapLayer)
        .layer(RateLimitHeaderLayer)
        .layer(cors_layer())
        .fallback_service(ServeDir::new(ui_dir.as_ref()))
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}

// ── Macro to reduce error-mapping boilerplate ───────────────────────────

/// Map a `shepherd_core::Error` to the matching response enum variant.
/// Response enums with NotFound + Conflict variants.
///
/// Note: 410 (`Error::LeaseExpired`) currently folds into Conflict. No S4
/// endpoint can produce it; when S5 adds claim endpoints, the spec (and the
/// generated enums) should gain a 410 Gone response so leases map correctly.
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
}

// ── RelationsApi ────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl RelationsApi for AppState {
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
