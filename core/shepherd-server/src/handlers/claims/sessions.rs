use shepherd_core::error::DomainError;
use shepherd_core::model::{ProjectId, TaskId};

use crate::AppState;
use crate::convert::errors::{
    domain_problem, parse_idempotency_key, parse_path_uuid, require_principal, require_store,
};
use crate::convert::{command_context, content, list_params, page, request_hash, work};
use crate::generated::server::api::SessionsApi;
use crate::generated::server::errors::{ListTaskSessionsResponse, ReportClaimResponse};
use crate::generated::types as wire;

#[async_trait::async_trait]
impl SessionsApi for AppState {
    async fn report_claim(
        &self,
        project_id: String,
        claim_id: String,
        x_lease_token: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReportInput,
    ) -> ReportClaimResponse {
        let store = require_store!(self, ReportClaimResponse);
        let principal = require_principal!(ReportClaimResponse);
        let project = parse_path_uuid!(ReportClaimResponse, project_id, "project_id");
        let claim = parse_path_uuid!(ReportClaimResponse, claim_id, "claim_id");
        let key = parse_idempotency_key!(ReportClaimResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/claims/{claim}/report"),
            &body,
        );
        let ctx = command_context(&principal, key, None, None, store);
        let outcome = async {
            let input = work::core_report_input(body)?;
            let session = store
                .report_claim(ctx, project, claim, &x_lease_token, input, &hash)
                .await?;
            Ok::<_, DomainError>(content::session(session.into_inner()))
        }
        .await;
        match outcome {
            Ok(body) => ReportClaimResponse::Created(body),
            Err(err) => domain_problem!(ReportClaimResponse, err),
        }
    }

    async fn list_task_sessions(
        &self,
        project_id: String,
        task_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListTaskSessionsResponse {
        let store = require_store!(self, ListTaskSessionsResponse);
        require_principal!(ListTaskSessionsResponse);
        let project = parse_path_uuid!(ListTaskSessionsResponse, project_id, "project_id");
        let task = parse_path_uuid!(ListTaskSessionsResponse, task_id, "task_id");
        match store
            .list_task_sessions(
                &ProjectId::from_uuid(project),
                &TaskId::from_uuid(task),
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, content::session);
                ListTaskSessionsResponse::Ok(wire::PageSession { items, next_cursor })
            }
            Err(err) => domain_problem!(ListTaskSessionsResponse, err),
        }
    }
}
