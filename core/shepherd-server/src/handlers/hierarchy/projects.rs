use shepherd_core::model::ProjectId;

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params, page};
use crate::generated::server::api::ProjectsApi;
use crate::generated::server::errors::{
    ArchiveProjectResponse, CreateProjectResponse, GetProjectResponse, ListProjectsResponse,
    UpdateProjectResponse,
};
use crate::generated::types as wire;
use crate::middleware::set_etag;

#[async_trait::async_trait]
impl ProjectsApi for AppState {
    async fn list_projects(
        &self,
        cursor: Option<String>,
        limit: Option<i32>,
        include_archived: Option<bool>,
    ) -> ListProjectsResponse {
        let store = require_store!(self, ListProjectsResponse);
        require_principal!(ListProjectsResponse);
        match store
            .list_projects(&list_params(cursor, limit, include_archived))
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, hierarchy::project);
                ListProjectsResponse::Ok(wire::PageProject { items, next_cursor })
            }
            Err(err) => domain_problem!(ListProjectsResponse, err),
        }
    }

    async fn create_project(
        &self,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ProjectCreate,
    ) -> CreateProjectResponse {
        let store = require_store!(self, CreateProjectResponse);
        let principal = require_principal!(CreateProjectResponse);
        let key = parse_idempotency_key!(CreateProjectResponse, idempotency_key);
        let ctx = command_context(&principal, key, None, store);
        match store
            .create_project(ctx, hierarchy::core_project_create(body))
            .await
        {
            Ok(result) => CreateProjectResponse::Created(hierarchy::project(result.value)),
            Err(err) => command_problem!(CreateProjectResponse, err),
        }
    }

    async fn get_project(&self, project_id: String) -> GetProjectResponse {
        let store = require_store!(self, GetProjectResponse);
        require_principal!(GetProjectResponse);
        let id = parse_path_uuid!(GetProjectResponse, project_id, "project_id");
        match store.get_project(&ProjectId::from_uuid(id)).await {
            Ok(found) => {
                set_etag(found.revision.value());
                GetProjectResponse::Ok(hierarchy::project(found))
            }
            Err(err) => domain_problem!(GetProjectResponse, err),
        }
    }

    async fn update_project(
        &self,
        project_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ProjectPatch,
    ) -> UpdateProjectResponse {
        let store = require_store!(self, UpdateProjectResponse);
        let principal = require_principal!(UpdateProjectResponse);
        let id = parse_path_uuid!(UpdateProjectResponse, project_id, "project_id");
        let revision = parse_revision!(UpdateProjectResponse, if_match);
        let key = parse_idempotency_key!(UpdateProjectResponse, idempotency_key);
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .update_project(
                ctx,
                ProjectId::from_uuid(id),
                hierarchy::core_project_patch(body),
            )
            .await
        {
            Ok(result) => UpdateProjectResponse::Ok(hierarchy::project(result.value)),
            Err(err) => revision_problem!(UpdateProjectResponse, err),
        }
    }

    async fn archive_project(
        &self,
        project_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> ArchiveProjectResponse {
        let store = require_store!(self, ArchiveProjectResponse);
        let principal = require_principal!(ArchiveProjectResponse);
        let id = parse_path_uuid!(ArchiveProjectResponse, project_id, "project_id");
        let revision = parse_revision!(ArchiveProjectResponse, if_match);
        let key = parse_idempotency_key!(ArchiveProjectResponse, idempotency_key);
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .archive_project(ctx, ProjectId::from_uuid(id), body.reason)
            .await
        {
            Ok(result) => ArchiveProjectResponse::Ok(hierarchy::project(result.value)),
            Err(err) => revision_problem!(ArchiveProjectResponse, err),
        }
    }
}
