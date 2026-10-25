use shepherd_core::model::{ProjectId, ReviewPolicy, SubmissionKind, SubmissionStatus, TaskId};
use shepherd_core::queries::SubmissionListFilters;

use crate::AppState;
use crate::convert::errors::{
    domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision, require_principal,
    require_store, revision_problem,
};
use crate::convert::{command_context, content, list_params, page, request_hash};
use crate::generated::server::api::{ListSubmissionsKind, ListSubmissionsStatus, ReviewsApi};
use crate::generated::server::errors::{
    GetSubmissionResponse, ListSubmissionReviewsResponse, ListSubmissionsResponse,
    ReviewSubmissionResponse, WithdrawSubmissionResponse,
};
use crate::generated::types as wire;
use crate::middleware::set_etag;

#[async_trait::async_trait]
impl ReviewsApi for AppState {
    async fn list_submissions(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        task_id: Option<String>,
        status: Option<ListSubmissionsStatus>,
        kind: Option<ListSubmissionsKind>,
        policy: Option<wire::ReviewPolicy>,
    ) -> ListSubmissionsResponse {
        let store = require_store!(self, ListSubmissionsResponse);
        require_principal!(ListSubmissionsResponse);
        let project = parse_path_uuid!(ListSubmissionsResponse, project_id, "project_id");
        let task_id = match task_id {
            Some(value) => Some(TaskId::from_uuid(parse_path_uuid!(
                ListSubmissionsResponse,
                value,
                "task_id"
            ))),
            None => None,
        };
        let filters = SubmissionListFilters {
            task_id,
            status: status.map(|status| match status {
                ListSubmissionsStatus::Pending => SubmissionStatus::Pending,
                ListSubmissionsStatus::Accepted => SubmissionStatus::Accepted,
                ListSubmissionsStatus::Rejected => SubmissionStatus::Rejected,
                ListSubmissionsStatus::Withdrawn => SubmissionStatus::Withdrawn,
            }),
            kind: kind.map(|kind| match kind {
                ListSubmissionsKind::Plan => SubmissionKind::Plan,
                ListSubmissionsKind::Work => SubmissionKind::Work,
            }),
            policy: policy.map(|policy| match policy {
                wire::ReviewPolicy::Human => ReviewPolicy::Human,
                wire::ReviewPolicy::Agent => ReviewPolicy::Agent,
                wire::ReviewPolicy::None => ReviewPolicy::None,
            }),
        };
        match store
            .list_submissions(
                &ProjectId::from_uuid(project),
                &filters,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, content::submission);
                ListSubmissionsResponse::Ok(wire::PageSubmission { items, next_cursor })
            }
            Err(err) => domain_problem!(ListSubmissionsResponse, err),
        }
    }

    async fn get_submission(
        &self,
        project_id: String,
        submission_id: String,
    ) -> GetSubmissionResponse {
        let store = require_store!(self, GetSubmissionResponse);
        require_principal!(GetSubmissionResponse);
        let project = parse_path_uuid!(GetSubmissionResponse, project_id, "project_id");
        let submission = parse_path_uuid!(GetSubmissionResponse, submission_id, "submission_id");
        match store
            .get_submission(&ProjectId::from_uuid(project), &submission)
            .await
        {
            Ok(found) => {
                set_etag(found.revision.value());
                GetSubmissionResponse::Ok(content::submission(found))
            }
            Err(err) => domain_problem!(GetSubmissionResponse, err),
        }
    }

    async fn withdraw_submission(
        &self,
        project_id: String,
        submission_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::ReasonInput,
    ) -> WithdrawSubmissionResponse {
        let store = require_store!(self, WithdrawSubmissionResponse);
        let principal = require_principal!(WithdrawSubmissionResponse);
        let project = parse_path_uuid!(WithdrawSubmissionResponse, project_id, "project_id");
        let submission =
            parse_path_uuid!(WithdrawSubmissionResponse, submission_id, "submission_id");
        let revision = parse_revision!(WithdrawSubmissionResponse, if_match);
        let key = parse_idempotency_key!(WithdrawSubmissionResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/submissions/{submission}/withdraw"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .withdraw_submission(ctx, project, submission, body.reason, &hash)
            .await
        {
            Ok(withdrawn) => {
                WithdrawSubmissionResponse::Ok(content::submission(withdrawn.into_inner()))
            }
            Err(err) => revision_problem!(WithdrawSubmissionResponse, err),
        }
    }

    async fn review_submission(
        &self,
        project_id: String,
        submission_id: String,
        if_match: String,
        idempotency_key: String,
        x_lease_token: Option<String>,
        _x_csrf_token: Option<String>,
        body: wire::ReviewInput,
    ) -> ReviewSubmissionResponse {
        let store = require_store!(self, ReviewSubmissionResponse);
        let principal = require_principal!(ReviewSubmissionResponse);
        let project = parse_path_uuid!(ReviewSubmissionResponse, project_id, "project_id");
        let submission = parse_path_uuid!(ReviewSubmissionResponse, submission_id, "submission_id");
        let revision = parse_revision!(ReviewSubmissionResponse, if_match);
        let key = parse_idempotency_key!(ReviewSubmissionResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/submissions/{submission}/reviews"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .review_submission(
                ctx,
                project,
                submission,
                x_lease_token.as_deref(),
                content::core_review_input(body),
                &hash,
            )
            .await
        {
            Ok(review) => ReviewSubmissionResponse::Created(content::review(review.into_inner())),
            Err(err) => revision_problem!(ReviewSubmissionResponse, err),
        }
    }

    async fn list_submission_reviews(
        &self,
        project_id: String,
        submission_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListSubmissionReviewsResponse {
        let store = require_store!(self, ListSubmissionReviewsResponse);
        require_principal!(ListSubmissionReviewsResponse);
        let project = parse_path_uuid!(ListSubmissionReviewsResponse, project_id, "project_id");
        let submission = parse_path_uuid!(
            ListSubmissionReviewsResponse,
            submission_id,
            "submission_id"
        );
        match store
            .list_submission_reviews(
                &ProjectId::from_uuid(project),
                &submission,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, content::review);
                ListSubmissionReviewsResponse::Ok(wire::PageReview { items, next_cursor })
            }
            Err(err) => domain_problem!(ListSubmissionReviewsResponse, err),
        }
    }
}
