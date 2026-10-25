use shepherd_core::error::DomainError;
use shepherd_core::model::{Actor, EpicId, ProjectId, TaskId};
use shepherd_core::queries::TaskListFilters;
use shepherd_core::storage::Store;

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params};
use crate::generated::server::api::TasksApi;
use crate::generated::server::errors::{
    AcceptTaskResponse, ArchiveTaskResponse, BlockTaskResponse, CancelTaskResponse,
    CreateTaskResponse, GetTaskResponse, ListTasksResponse, UnblockTaskResponse,
    UpdateTaskResponse, WaiveTaskResponse,
};
use crate::generated::types as wire;
use crate::middleware::set_etag;

/// Wire body for one task: the post-commit view carrying eligibility and the
/// plan-pointer arrays.
async fn task_body(
    store: &Store,
    project: ProjectId,
    task: TaskId,
    principal: &Actor,
) -> Result<wire::Task, DomainError> {
    Ok(hierarchy::task(
        store.task_view(&project, &task, principal).await?,
    ))
}

#[async_trait::async_trait]
impl TasksApi for AppState {
    async fn list_tasks(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        epic_id: Option<String>,
        status: Option<wire::EntityStatus>,
        phase: Option<wire::TaskPhase>,
        type_key: Option<String>,
        include_archived: Option<bool>,
    ) -> ListTasksResponse {
        let store = require_store!(self, ListTasksResponse);
        let principal = require_principal!(ListTasksResponse);
        let project = parse_path_uuid!(ListTasksResponse, project_id, "project_id");
        let project = ProjectId::from_uuid(project);
        let epic_id = match epic_id {
            Some(value) => Some(EpicId::from_uuid(parse_path_uuid!(
                ListTasksResponse,
                value,
                "epic_id"
            ))),
            None => None,
        };
        let filters = TaskListFilters {
            epic_id,
            status: status.map(hierarchy::core_task_status),
            phase: phase.map(hierarchy::core_task_phase),
            type_key,
        };
        let outcome = async {
            let page = store
                .list_tasks(
                    &project,
                    &filters,
                    &list_params(cursor, limit, include_archived),
                )
                .await?;
            let ids: Vec<TaskId> = page.items.iter().map(|task| task.id).collect();
            let views = store.task_views(&project, &ids, &principal).await?;
            Ok::<_, DomainError>(wire::PageTask {
                items: views.into_iter().map(hierarchy::task).collect(),
                next_cursor: page.next_cursor,
            })
        }
        .await;
        match outcome {
            Ok(body) => ListTasksResponse::Ok(body),
            Err(err) => domain_problem!(ListTasksResponse, err),
        }
    }

    async fn create_task(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TaskCreate,
    ) -> CreateTaskResponse {
        let store = require_store!(self, CreateTaskResponse);
        let principal = require_principal!(CreateTaskResponse);
        let project = parse_path_uuid!(CreateTaskResponse, project_id, "project_id");
        let project = ProjectId::from_uuid(project);
        let key = parse_idempotency_key!(CreateTaskResponse, idempotency_key);
        let ctx = command_context(&principal, key, None, store);
        let (epic, input) = hierarchy::core_task_create(body);
        let outcome = async {
            let result = store.create_task(ctx, project, epic, input).await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => CreateTaskResponse::Created(body),
            Err(err) => command_problem!(CreateTaskResponse, err),
        }
    }

    async fn get_task(&self, project_id: String, task_id: String) -> GetTaskResponse {
        let store = require_store!(self, GetTaskResponse);
        let principal = require_principal!(GetTaskResponse);
        let project = parse_path_uuid!(GetTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(GetTaskResponse, task_id, "task_id");
        match task_body(
            store,
            ProjectId::from_uuid(project),
            TaskId::from_uuid(task),
            &principal,
        )
        .await
        {
            Ok(body) => {
                set_etag(i64::from(body.revision));
                GetTaskResponse::Ok(body)
            }
            Err(err) => domain_problem!(GetTaskResponse, err),
        }
    }

    async fn update_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TaskPatch,
    ) -> UpdateTaskResponse {
        let store = require_store!(self, UpdateTaskResponse);
        let principal = require_principal!(UpdateTaskResponse);
        let project = parse_path_uuid!(UpdateTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(UpdateTaskResponse, task_id, "task_id");
        let revision = parse_revision!(UpdateTaskResponse, if_match);
        let key = parse_idempotency_key!(UpdateTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .update_task(
                    ctx,
                    project,
                    TaskId::from_uuid(task),
                    hierarchy::core_task_patch(body),
                )
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => UpdateTaskResponse::Ok(body),
            Err(err) => revision_problem!(UpdateTaskResponse, err),
        }
    }

    async fn archive_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> ArchiveTaskResponse {
        let store = require_store!(self, ArchiveTaskResponse);
        let principal = require_principal!(ArchiveTaskResponse);
        let project = parse_path_uuid!(ArchiveTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(ArchiveTaskResponse, task_id, "task_id");
        let revision = parse_revision!(ArchiveTaskResponse, if_match);
        let key = parse_idempotency_key!(ArchiveTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .archive_task(ctx, project, TaskId::from_uuid(task), body.reason)
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => ArchiveTaskResponse::Ok(body),
            Err(err) => revision_problem!(ArchiveTaskResponse, err),
        }
    }

    async fn accept_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        _body: wire::EmptyInput,
    ) -> AcceptTaskResponse {
        let store = require_store!(self, AcceptTaskResponse);
        let principal = require_principal!(AcceptTaskResponse);
        let project = parse_path_uuid!(AcceptTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(AcceptTaskResponse, task_id, "task_id");
        let revision = parse_revision!(AcceptTaskResponse, if_match);
        let key = parse_idempotency_key!(AcceptTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .accept_task(ctx, project, TaskId::from_uuid(task))
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => AcceptTaskResponse::Ok(body),
            Err(err) => revision_problem!(AcceptTaskResponse, err),
        }
    }

    async fn block_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> BlockTaskResponse {
        let store = require_store!(self, BlockTaskResponse);
        let principal = require_principal!(BlockTaskResponse);
        let project = parse_path_uuid!(BlockTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(BlockTaskResponse, task_id, "task_id");
        let revision = parse_revision!(BlockTaskResponse, if_match);
        let key = parse_idempotency_key!(BlockTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .block_task(ctx, project, TaskId::from_uuid(task), body.reason)
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => BlockTaskResponse::Ok(body),
            Err(err) => revision_problem!(BlockTaskResponse, err),
        }
    }

    async fn unblock_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        _body: wire::EmptyInput,
    ) -> UnblockTaskResponse {
        let store = require_store!(self, UnblockTaskResponse);
        let principal = require_principal!(UnblockTaskResponse);
        let project = parse_path_uuid!(UnblockTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(UnblockTaskResponse, task_id, "task_id");
        let revision = parse_revision!(UnblockTaskResponse, if_match);
        let key = parse_idempotency_key!(UnblockTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .unblock_task(ctx, project, TaskId::from_uuid(task))
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => UnblockTaskResponse::Ok(body),
            Err(err) => revision_problem!(UnblockTaskResponse, err),
        }
    }

    async fn cancel_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> CancelTaskResponse {
        let store = require_store!(self, CancelTaskResponse);
        let principal = require_principal!(CancelTaskResponse);
        let project = parse_path_uuid!(CancelTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(CancelTaskResponse, task_id, "task_id");
        let revision = parse_revision!(CancelTaskResponse, if_match);
        let key = parse_idempotency_key!(CancelTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .cancel_task(ctx, project, TaskId::from_uuid(task), body.reason)
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => CancelTaskResponse::Ok(body),
            Err(err) => revision_problem!(CancelTaskResponse, err),
        }
    }

    async fn waive_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> WaiveTaskResponse {
        let store = require_store!(self, WaiveTaskResponse);
        let principal = require_principal!(WaiveTaskResponse);
        let project = parse_path_uuid!(WaiveTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(WaiveTaskResponse, task_id, "task_id");
        let revision = parse_revision!(WaiveTaskResponse, if_match);
        let key = parse_idempotency_key!(WaiveTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .waive_task(ctx, project, TaskId::from_uuid(task), body.reason)
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => WaiveTaskResponse::Ok(body),
            Err(err) => revision_problem!(WaiveTaskResponse, err),
        }
    }
}
