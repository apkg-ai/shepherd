use shepherd_core::error::DomainError;
use shepherd_core::model::{ClaimStatus, ProjectId, TaskId};
use shepherd_core::queries::ClaimListFilters;

use crate::AppState;
use crate::convert::errors::{
    domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision, require_principal,
    require_store, revision_problem,
};
use crate::convert::{command_context, list_params, page, request_hash, work};
use crate::generated::server::api::ClaimsApi;
use crate::generated::server::errors::{
    ClaimTaskResponse, ListClaimsResponse, ReleaseClaimResponse, RenewClaimResponse,
};
use crate::generated::types as wire;

use super::context_body;

#[async_trait::async_trait]
impl ClaimsApi for AppState {
    async fn claim_task(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ClaimInput,
    ) -> ClaimTaskResponse {
        let store = require_store!(self, ClaimTaskResponse);
        let principal = require_principal!(ClaimTaskResponse);
        let project = parse_path_uuid!(ClaimTaskResponse, project_id, "project_id");
        let task = parse_path_uuid!(ClaimTaskResponse, task_id, "task_id");
        let revision = parse_revision!(ClaimTaskResponse, if_match);
        let key = parse_idempotency_key!(ClaimTaskResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let task = TaskId::from_uuid(task);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/tasks/{task}/claims"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            // A replayed claim returns the original lease even if expired
            // (plan/07); the context is assembled fresh either way.
            let grant = store
                .claim_task(ctx, project, task, work::core_claim_input(body), &hash)
                .await?
                .into_inner();
            let context = context_body(store, project, task, &principal).await?;
            Ok::<_, DomainError>(wire::ClaimGrant {
                claim: work::claim(grant.claim),
                lease_token: grant.lease_token,
                context,
            })
        }
        .await;
        match outcome {
            Ok(body) => ClaimTaskResponse::Created(body),
            Err(err) => revision_problem!(ClaimTaskResponse, err),
        }
    }

    async fn list_claims(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        task_id: Option<String>,
        active: Option<bool>,
    ) -> ListClaimsResponse {
        let store = require_store!(self, ListClaimsResponse);
        require_principal!(ListClaimsResponse);
        let project = parse_path_uuid!(ListClaimsResponse, project_id, "project_id");
        let task_id = match task_id {
            Some(value) => Some(TaskId::from_uuid(parse_path_uuid!(
                ListClaimsResponse,
                value,
                "task_id"
            ))),
            None => None,
        };
        // active=true filters unexpired active rows; default includes all (plan/07).
        let filters = ClaimListFilters {
            task_id,
            status: (active == Some(true)).then_some(ClaimStatus::Active),
            phase: None,
        };
        match store
            .list_claims(
                &ProjectId::from_uuid(project),
                &filters,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, work::claim);
                ListClaimsResponse::Ok(wire::PageClaim { items, next_cursor })
            }
            Err(err) => domain_problem!(ListClaimsResponse, err),
        }
    }

    async fn renew_claim(
        &self,
        project_id: String,
        claim_id: String,
        x_lease_token: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::RenewInput,
    ) -> RenewClaimResponse {
        let store = require_store!(self, RenewClaimResponse);
        let principal = require_principal!(RenewClaimResponse);
        let project = parse_path_uuid!(RenewClaimResponse, project_id, "project_id");
        let claim = parse_path_uuid!(RenewClaimResponse, claim_id, "claim_id");
        let key = parse_idempotency_key!(RenewClaimResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/claims/{claim}/renew"),
            &body,
        );
        let ctx = command_context(&principal, key, None, store);
        match store
            .renew_claim(
                ctx,
                project,
                claim,
                &x_lease_token,
                work::core_renew_input(body),
                &hash,
            )
            .await
        {
            Ok(renewed) => RenewClaimResponse::Ok(work::claim(renewed.into_inner())),
            Err(err) => domain_problem!(RenewClaimResponse, err),
        }
    }

    async fn release_claim(
        &self,
        project_id: String,
        claim_id: String,
        x_lease_token: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        _body: wire::EmptyInput,
    ) -> ReleaseClaimResponse {
        let store = require_store!(self, ReleaseClaimResponse);
        let principal = require_principal!(ReleaseClaimResponse);
        let project = parse_path_uuid!(ReleaseClaimResponse, project_id, "project_id");
        let claim = parse_path_uuid!(ReleaseClaimResponse, claim_id, "claim_id");
        let key = parse_idempotency_key!(ReleaseClaimResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/claims/{claim}/release"),
            &serde_json::json!({}),
        );
        let ctx = command_context(&principal, key, None, store);
        match store
            .release_claim(ctx, project, claim, &x_lease_token, &hash)
            .await
        {
            Ok(released) => ReleaseClaimResponse::Ok(work::claim(released.into_inner())),
            Err(err) => domain_problem!(ReleaseClaimResponse, err),
        }
    }
}
