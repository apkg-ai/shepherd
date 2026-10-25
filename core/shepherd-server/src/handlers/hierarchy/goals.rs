use shepherd_core::model::{GoalId, ProjectId};

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, hierarchy, list_params, page};
use crate::generated::server::api::GoalsApi;
use crate::generated::server::errors::{
    ArchiveGoalResponse, CreateGoalResponse, GetGoalResponse, ListGoalsResponse, UpdateGoalResponse,
};
use crate::generated::types as wire;
use crate::middleware::set_etag;

#[async_trait::async_trait]
impl GoalsApi for AppState {
    async fn list_goals(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        include_archived: Option<bool>,
    ) -> ListGoalsResponse {
        let store = require_store!(self, ListGoalsResponse);
        require_principal!(ListGoalsResponse);
        let project = parse_path_uuid!(ListGoalsResponse, project_id, "project_id");
        match store
            .list_goals(
                &ProjectId::from_uuid(project),
                &list_params(cursor, limit, include_archived),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, hierarchy::goal);
                ListGoalsResponse::Ok(wire::PageGoal { items, next_cursor })
            }
            Err(err) => domain_problem!(ListGoalsResponse, err),
        }
    }

    async fn create_goal(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::GoalCreate,
    ) -> CreateGoalResponse {
        let store = require_store!(self, CreateGoalResponse);
        let principal = require_principal!(CreateGoalResponse);
        let project = parse_path_uuid!(CreateGoalResponse, project_id, "project_id");
        let key = parse_idempotency_key!(CreateGoalResponse, idempotency_key);
        let ctx = command_context(&principal, key, None, store);
        match store
            .create_goal(
                ctx,
                ProjectId::from_uuid(project),
                hierarchy::core_goal_create(body),
            )
            .await
        {
            Ok(result) => CreateGoalResponse::Created(hierarchy::goal(result.value)),
            Err(err) => command_problem!(CreateGoalResponse, err),
        }
    }

    async fn get_goal(&self, project_id: String, goal_id: String) -> GetGoalResponse {
        let store = require_store!(self, GetGoalResponse);
        require_principal!(GetGoalResponse);
        let project = parse_path_uuid!(GetGoalResponse, project_id, "project_id");
        let goal = parse_path_uuid!(GetGoalResponse, goal_id, "goal_id");
        match store
            .get_goal(&ProjectId::from_uuid(project), &GoalId::from_uuid(goal))
            .await
        {
            Ok(found) => {
                set_etag(found.revision.value());
                GetGoalResponse::Ok(hierarchy::goal(found))
            }
            Err(err) => domain_problem!(GetGoalResponse, err),
        }
    }

    async fn update_goal(
        &self,
        project_id: String,
        goal_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::TextPatch,
    ) -> UpdateGoalResponse {
        let store = require_store!(self, UpdateGoalResponse);
        let principal = require_principal!(UpdateGoalResponse);
        let project = parse_path_uuid!(UpdateGoalResponse, project_id, "project_id");
        let goal = parse_path_uuid!(UpdateGoalResponse, goal_id, "goal_id");
        let revision = parse_revision!(UpdateGoalResponse, if_match);
        let key = parse_idempotency_key!(UpdateGoalResponse, idempotency_key);
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .update_goal(
                ctx,
                ProjectId::from_uuid(project),
                GoalId::from_uuid(goal),
                hierarchy::core_text_patch(body),
            )
            .await
        {
            Ok(result) => UpdateGoalResponse::Ok(hierarchy::goal(result.value)),
            Err(err) => revision_problem!(UpdateGoalResponse, err),
        }
    }

    async fn archive_goal(
        &self,
        project_id: String,
        goal_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> ArchiveGoalResponse {
        let store = require_store!(self, ArchiveGoalResponse);
        let principal = require_principal!(ArchiveGoalResponse);
        let project = parse_path_uuid!(ArchiveGoalResponse, project_id, "project_id");
        let goal = parse_path_uuid!(ArchiveGoalResponse, goal_id, "goal_id");
        let revision = parse_revision!(ArchiveGoalResponse, if_match);
        let key = parse_idempotency_key!(ArchiveGoalResponse, idempotency_key);
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .archive_goal(
                ctx,
                ProjectId::from_uuid(project),
                GoalId::from_uuid(goal),
                body.reason,
            )
            .await
        {
            Ok(result) => ArchiveGoalResponse::Ok(hierarchy::goal(result.value)),
            Err(err) => revision_problem!(ArchiveGoalResponse, err),
        }
    }
}
