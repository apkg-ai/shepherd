use shepherd_core::model::{ProjectId, TaskTypeId};

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params, page};
use crate::generated::server::api::TaskTypesApi;
use crate::generated::server::errors::{
    CreateTaskTypeResponse, ListTaskTypesResponse, UpdateTaskTypeResponse,
};
use crate::generated::types as wire;

#[async_trait::async_trait]
impl TaskTypesApi for AppState {
    async fn list_task_types(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        include_archived: Option<bool>,
    ) -> ListTaskTypesResponse {
        let store = require_store!(self, ListTaskTypesResponse);
        require_principal!(ListTaskTypesResponse);
        let project = parse_path_uuid!(ListTaskTypesResponse, project_id, "project_id");
        match store
            .list_task_types(
                &ProjectId::from_uuid(project),
                &list_params(cursor, limit, include_archived),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, hierarchy::task_type);
                ListTaskTypesResponse::Ok(wire::PageTaskType { items, next_cursor })
            }
            Err(err) => domain_problem!(ListTaskTypesResponse, err),
        }
    }

    async fn create_task_type(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TaskTypeCreate,
    ) -> CreateTaskTypeResponse {
        let store = require_store!(self, CreateTaskTypeResponse);
        let principal = require_principal!(CreateTaskTypeResponse);
        let project = parse_path_uuid!(CreateTaskTypeResponse, project_id, "project_id");
        let key = parse_idempotency_key!(CreateTaskTypeResponse, idempotency_key);
        let ctx = command_context(&principal, key, None, store);
        match store
            .create_task_type(
                ctx,
                ProjectId::from_uuid(project),
                hierarchy::core_task_type_create(body),
            )
            .await
        {
            Ok(result) => CreateTaskTypeResponse::Created(hierarchy::task_type(result.value)),
            Err(err) => command_problem!(CreateTaskTypeResponse, err),
        }
    }

    async fn update_task_type(
        &self,
        project_id: String,
        type_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TaskTypePatch,
    ) -> UpdateTaskTypeResponse {
        let store = require_store!(self, UpdateTaskTypeResponse);
        let principal = require_principal!(UpdateTaskTypeResponse);
        let project = parse_path_uuid!(UpdateTaskTypeResponse, project_id, "project_id");
        let task_type = parse_path_uuid!(UpdateTaskTypeResponse, type_id, "type_id");
        let revision = parse_revision!(UpdateTaskTypeResponse, if_match);
        let key = parse_idempotency_key!(UpdateTaskTypeResponse, idempotency_key);
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .update_task_type(
                ctx,
                ProjectId::from_uuid(project),
                TaskTypeId::from_uuid(task_type),
                hierarchy::core_task_type_patch(body),
            )
            .await
        {
            Ok(result) => UpdateTaskTypeResponse::Ok(hierarchy::task_type(result.value)),
            Err(err) => revision_problem!(UpdateTaskTypeResponse, err),
        }
    }
}
