use shepherd_core::error::DomainError;
use shepherd_core::model::{Actor, EpicId, GoalId, ProjectId};
use shepherd_core::queries::EpicListFilters;
use shepherd_core::storage::Store;

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params, request_hash};
use crate::generated::server::api::EpicsApi;
use crate::generated::server::errors::{
    AcceptEpicResponse, ArchiveEpicResponse, BlockEpicResponse, CancelEpicResponse,
    CompleteEpicResponse, CreateEpicResponse, GetEpicGraphResponse, GetEpicResponse,
    ListEpicsResponse, UnblockEpicResponse, UpdateEpicResponse,
};
use crate::generated::types as wire;
use crate::middleware::set_etag;

/// Wire body for one epic: the post-commit view carrying eligibility.
async fn epic_body(
    store: &Store,
    project: ProjectId,
    epic: EpicId,
    principal: &Actor,
) -> Result<wire::Epic, DomainError> {
    Ok(hierarchy::epic(
        store.epic_view(&project, &epic, principal).await?,
    ))
}

#[async_trait::async_trait]
impl EpicsApi for AppState {
    async fn list_epics(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        goal_id: Option<String>,
        include_archived: Option<bool>,
        status: Option<wire::EntityStatus>,
    ) -> ListEpicsResponse {
        let store = require_store!(self, ListEpicsResponse);
        let principal = require_principal!(ListEpicsResponse);
        let project = parse_path_uuid!(ListEpicsResponse, project_id, "project_id");
        let project = ProjectId::from_uuid(project);
        let goal_id = match goal_id {
            Some(value) => Some(GoalId::from_uuid(parse_path_uuid!(
                ListEpicsResponse,
                value,
                "goal_id"
            ))),
            None => None,
        };
        let filters = EpicListFilters {
            goal_id,
            status: status.map(hierarchy::core_epic_status),
        };
        let outcome = async {
            let page = store
                .list_epics(
                    &project,
                    &filters,
                    &list_params(cursor, limit, include_archived),
                )
                .await?;
            let ids: Vec<EpicId> = page.items.iter().map(|epic| epic.id).collect();
            let views = store.epic_views(&project, &ids, &principal).await?;
            Ok::<_, DomainError>(wire::PageEpic {
                items: views.into_iter().map(hierarchy::epic).collect(),
                next_cursor: page.next_cursor,
            })
        }
        .await;
        match outcome {
            Ok(body) => ListEpicsResponse::Ok(body),
            Err(err) => domain_problem!(ListEpicsResponse, err),
        }
    }

    async fn create_epic(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::EpicCreate,
    ) -> CreateEpicResponse {
        let store = require_store!(self, CreateEpicResponse);
        let principal = require_principal!(CreateEpicResponse);
        let project = parse_path_uuid!(CreateEpicResponse, project_id, "project_id");
        let project = ProjectId::from_uuid(project);
        let key = parse_idempotency_key!(CreateEpicResponse, idempotency_key);
        let hash = request_hash("POST", &format!("/api/v1/projects/{project}/epics"), &body);
        let ctx = command_context(&principal, key, None, Some(hash), store);
        let (goal, input) = hierarchy::core_epic_create(body);
        let outcome = async {
            let result = store.create_epic(ctx, project, goal, input).await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => CreateEpicResponse::Created(body),
            Err(err) => command_problem!(CreateEpicResponse, err),
        }
    }

    async fn get_epic(&self, project_id: String, epic_id: String) -> GetEpicResponse {
        let store = require_store!(self, GetEpicResponse);
        let principal = require_principal!(GetEpicResponse);
        let project = parse_path_uuid!(GetEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(GetEpicResponse, epic_id, "epic_id");
        match epic_body(
            store,
            ProjectId::from_uuid(project),
            EpicId::from_uuid(epic),
            &principal,
        )
        .await
        {
            Ok(body) => {
                set_etag(i64::from(body.revision));
                GetEpicResponse::Ok(body)
            }
            Err(err) => domain_problem!(GetEpicResponse, err),
        }
    }

    async fn update_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TextPatch,
    ) -> UpdateEpicResponse {
        let store = require_store!(self, UpdateEpicResponse);
        let principal = require_principal!(UpdateEpicResponse);
        let project = parse_path_uuid!(UpdateEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(UpdateEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(UpdateEpicResponse, if_match);
        let key = parse_idempotency_key!(UpdateEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "PATCH",
            &format!("/api/v1/projects/{project}/epics/{epic}"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .update_epic(
                    ctx,
                    project,
                    EpicId::from_uuid(epic),
                    hierarchy::core_text_patch(body),
                )
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => UpdateEpicResponse::Ok(body),
            Err(err) => revision_problem!(UpdateEpicResponse, err),
        }
    }

    async fn archive_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> ArchiveEpicResponse {
        let store = require_store!(self, ArchiveEpicResponse);
        let principal = require_principal!(ArchiveEpicResponse);
        let project = parse_path_uuid!(ArchiveEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(ArchiveEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(ArchiveEpicResponse, if_match);
        let key = parse_idempotency_key!(ArchiveEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/archive"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .archive_epic(ctx, project, EpicId::from_uuid(epic), body.reason)
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => ArchiveEpicResponse::Ok(body),
            Err(err) => revision_problem!(ArchiveEpicResponse, err),
        }
    }

    async fn accept_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        _body: wire::EmptyInput,
    ) -> AcceptEpicResponse {
        let store = require_store!(self, AcceptEpicResponse);
        let principal = require_principal!(AcceptEpicResponse);
        let project = parse_path_uuid!(AcceptEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(AcceptEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(AcceptEpicResponse, if_match);
        let key = parse_idempotency_key!(AcceptEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/accept"),
            &serde_json::json!({}),
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .accept_epic(ctx, project, EpicId::from_uuid(epic))
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => AcceptEpicResponse::Ok(body),
            Err(err) => revision_problem!(AcceptEpicResponse, err),
        }
    }

    async fn block_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> BlockEpicResponse {
        let store = require_store!(self, BlockEpicResponse);
        let principal = require_principal!(BlockEpicResponse);
        let project = parse_path_uuid!(BlockEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(BlockEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(BlockEpicResponse, if_match);
        let key = parse_idempotency_key!(BlockEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/block"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .block_epic(ctx, project, EpicId::from_uuid(epic), body.reason)
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => BlockEpicResponse::Ok(body),
            Err(err) => revision_problem!(BlockEpicResponse, err),
        }
    }

    async fn unblock_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        _body: wire::EmptyInput,
    ) -> UnblockEpicResponse {
        let store = require_store!(self, UnblockEpicResponse);
        let principal = require_principal!(UnblockEpicResponse);
        let project = parse_path_uuid!(UnblockEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(UnblockEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(UnblockEpicResponse, if_match);
        let key = parse_idempotency_key!(UnblockEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/unblock"),
            &serde_json::json!({}),
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .unblock_epic(ctx, project, EpicId::from_uuid(epic))
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => UnblockEpicResponse::Ok(body),
            Err(err) => revision_problem!(UnblockEpicResponse, err),
        }
    }

    async fn cancel_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> CancelEpicResponse {
        let store = require_store!(self, CancelEpicResponse);
        let principal = require_principal!(CancelEpicResponse);
        let project = parse_path_uuid!(CancelEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(CancelEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(CancelEpicResponse, if_match);
        let key = parse_idempotency_key!(CancelEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/cancel"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .cancel_epic(ctx, project, EpicId::from_uuid(epic), body.reason)
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => CancelEpicResponse::Ok(body),
            Err(err) => revision_problem!(CancelEpicResponse, err),
        }
    }

    async fn complete_epic(
        &self,
        project_id: String,
        epic_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> CompleteEpicResponse {
        let store = require_store!(self, CompleteEpicResponse);
        let principal = require_principal!(CompleteEpicResponse);
        let project = parse_path_uuid!(CompleteEpicResponse, project_id, "project_id");
        let epic = parse_path_uuid!(CompleteEpicResponse, epic_id, "epic_id");
        let revision = parse_revision!(CompleteEpicResponse, if_match);
        let key = parse_idempotency_key!(CompleteEpicResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/epics/{epic}/complete"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), Some(hash), store);
        let outcome = async {
            let result = store
                .complete_epic(ctx, project, EpicId::from_uuid(epic), body.reason)
                .await?;
            epic_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => CompleteEpicResponse::Ok(body),
            Err(err) => revision_problem!(CompleteEpicResponse, err),
        }
    }

    async fn get_epic_graph(&self, project_id: String, epic_id: String) -> GetEpicGraphResponse {
        let store = require_store!(self, GetEpicGraphResponse);
        let principal = require_principal!(GetEpicGraphResponse);
        let project = parse_path_uuid!(GetEpicGraphResponse, project_id, "project_id");
        let epic = parse_path_uuid!(GetEpicGraphResponse, epic_id, "epic_id");
        match store
            .get_epic_graph(
                &principal,
                &ProjectId::from_uuid(project),
                &EpicId::from_uuid(epic),
            )
            .await
        {
            Ok(graph) => GetEpicGraphResponse::Ok(hierarchy::graph(graph)),
            Err(err) => domain_problem!(GetEpicGraphResponse, err),
        }
    }
}
