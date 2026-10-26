use shepherd_core::model::{DependencyId, ProjectId};
use shepherd_core::queries::DependencyListFilters;

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params, page, request_hash};
use crate::generated::server::api::{DependenciesApi, ListDependenciesLevel};
use crate::generated::server::errors::{
    CreateDependencyResponse, DeleteDependencyResponse, ListDependenciesResponse,
};
use crate::generated::types as wire;

#[async_trait::async_trait]
impl DependenciesApi for AppState {
    async fn list_dependencies(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        level: Option<ListDependenciesLevel>,
        scope_id: Option<String>,
    ) -> ListDependenciesResponse {
        let store = require_store!(self, ListDependenciesResponse);
        require_principal!(ListDependenciesResponse);
        let project = parse_path_uuid!(ListDependenciesResponse, project_id, "project_id");
        let scope_id = match scope_id {
            Some(value) => Some(parse_path_uuid!(
                ListDependenciesResponse,
                value,
                "scope_id"
            )),
            None => None,
        };
        let filters = DependencyListFilters {
            level: level.map(hierarchy::core_dependency_level),
            scope_id,
        };
        match store
            .list_dependencies(
                &ProjectId::from_uuid(project),
                &filters,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, hierarchy::dependency);
                ListDependenciesResponse::Ok(wire::PageDependency { items, next_cursor })
            }
            Err(err) => domain_problem!(ListDependenciesResponse, err),
        }
    }

    async fn create_dependency(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::DependencyCreate,
    ) -> CreateDependencyResponse {
        let store = require_store!(self, CreateDependencyResponse);
        let principal = require_principal!(CreateDependencyResponse);
        let project = parse_path_uuid!(CreateDependencyResponse, project_id, "project_id");
        let key = parse_idempotency_key!(CreateDependencyResponse, idempotency_key);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/dependencies"),
            &body,
        );
        let ctx = command_context(&principal, key, None, Some(hash), store);
        match store
            .create_dependency(
                ctx,
                ProjectId::from_uuid(project),
                hierarchy::core_dependency_create(body),
            )
            .await
        {
            Ok(result) => CreateDependencyResponse::Created(hierarchy::dependency(result.value)),
            Err(err) => command_problem!(CreateDependencyResponse, err),
        }
    }

    async fn delete_dependency(
        &self,
        project_id: String,
        dependency_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
    ) -> DeleteDependencyResponse {
        let store = require_store!(self, DeleteDependencyResponse);
        let principal = require_principal!(DeleteDependencyResponse);
        let project = parse_path_uuid!(DeleteDependencyResponse, project_id, "project_id");
        let dependency = parse_path_uuid!(DeleteDependencyResponse, dependency_id, "dependency_id");
        let revision = parse_revision!(DeleteDependencyResponse, if_match);
        let key = parse_idempotency_key!(DeleteDependencyResponse, idempotency_key);
        let hash = request_hash(
            "DELETE",
            &format!("/api/v1/projects/{project}/dependencies/{dependency}"),
            &serde_json::json!({}),
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        match store
            .delete_dependency(
                ctx,
                ProjectId::from_uuid(project),
                DependencyId::from_uuid(dependency),
            )
            .await
        {
            Ok(_) => DeleteDependencyResponse::Ok(wire::Ack { ok: true }),
            Err(err) => revision_problem!(DeleteDependencyResponse, err),
        }
    }
}
