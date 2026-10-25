use shepherd_core::error::DomainError;
use shepherd_core::model::{EpicId, ProjectId, TaskId};
use shepherd_core::queries::WorkFilters;

use crate::AppState;
use crate::convert::errors::{domain_problem, parse_path_uuid, require_principal, require_store};
use crate::convert::{hierarchy, list_params, work};
use crate::generated::server::api::{ListWorkPhase, WorkApi};
use crate::generated::server::errors::{GetTaskContextResponse, ListWorkResponse};
use crate::generated::types as wire;

use super::context_body;

#[async_trait::async_trait]
impl WorkApi for AppState {
    async fn list_work(
        &self,
        project_id: String,
        phase: ListWorkPhase,
        cursor: Option<String>,
        limit: Option<i32>,
        epic_id: Option<String>,
        type_key: Option<String>,
    ) -> ListWorkResponse {
        let store = require_store!(self, ListWorkResponse);
        let principal = require_principal!(ListWorkResponse);
        let project = parse_path_uuid!(ListWorkResponse, project_id, "project_id");
        let project = ProjectId::from_uuid(project);
        let epic_id = match epic_id {
            Some(value) => Some(EpicId::from_uuid(parse_path_uuid!(
                ListWorkResponse,
                value,
                "epic_id"
            ))),
            None => None,
        };
        let filters = WorkFilters { epic_id, type_key };
        let outcome = async {
            let found = store
                .list_work(
                    &principal,
                    &project,
                    work::core_work_phase(phase),
                    &filters,
                    &list_params(cursor, limit, None),
                )
                .await?;
            // The advisory list is caller-scoped; the wire Task body needs the
            // caller's full view (plan pointers included), so re-read as views.
            let ids: Vec<TaskId> = found.items.iter().map(|item| item.task.id).collect();
            let views = store.task_views(&project, &ids, &principal).await?;
            Ok::<_, DomainError>(wire::PageTask {
                items: views.into_iter().map(hierarchy::task).collect(),
                next_cursor: found.next_cursor,
            })
        }
        .await;
        match outcome {
            Ok(body) => ListWorkResponse::Ok(body),
            Err(err) => domain_problem!(ListWorkResponse, err),
        }
    }

    async fn get_task_context(
        &self,
        project_id: String,
        task_id: String,
    ) -> GetTaskContextResponse {
        let store = require_store!(self, GetTaskContextResponse);
        let principal = require_principal!(GetTaskContextResponse);
        let project = parse_path_uuid!(GetTaskContextResponse, project_id, "project_id");
        let task = parse_path_uuid!(GetTaskContextResponse, task_id, "task_id");
        match context_body(
            store,
            ProjectId::from_uuid(project),
            TaskId::from_uuid(task),
            &principal,
        )
        .await
        {
            Ok(body) => GetTaskContextResponse::Ok(body),
            Err(err) => domain_problem!(GetTaskContextResponse, err),
        }
    }
}
