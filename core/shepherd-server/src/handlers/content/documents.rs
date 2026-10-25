use shepherd_core::commands::DocumentListFilters;
use shepherd_core::model::{
    DocumentId, DocumentKind, DocumentOwnerKind, DocumentRevisionId, ProjectId, TaskId,
};

use crate::AppState;
use crate::convert::errors::{
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_store, revision_problem,
};
use crate::convert::{command_context, content, list_params, page, request_hash};
use crate::generated::server::api::{DocumentsApi, ListDocumentsKind, ListDocumentsOwnerKind};
use crate::generated::server::errors::{
    CreateDocumentResponse, CreateDocumentRevisionResponse, GetDocumentResponse,
    GetDocumentRevisionResponse, ListDocumentRevisionsResponse, ListDocumentsResponse,
    SelectTaskPlanResponse,
};
use crate::generated::types as wire;
use crate::handlers::hierarchy::tasks::task_body;
use crate::middleware::set_etag;

#[async_trait::async_trait]
impl DocumentsApi for AppState {
    async fn list_documents(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        owner_kind: Option<ListDocumentsOwnerKind>,
        owner_id: Option<String>,
        kind: Option<ListDocumentsKind>,
    ) -> ListDocumentsResponse {
        let store = require_store!(self, ListDocumentsResponse);
        require_principal!(ListDocumentsResponse);
        let project = parse_path_uuid!(ListDocumentsResponse, project_id, "project_id");
        let owner_id = match owner_id {
            Some(value) => Some(parse_path_uuid!(ListDocumentsResponse, value, "owner_id")),
            None => None,
        };
        let filters = DocumentListFilters {
            owner_kind: owner_kind.map(|kind| match kind {
                ListDocumentsOwnerKind::Project => DocumentOwnerKind::Project,
                ListDocumentsOwnerKind::Epic => DocumentOwnerKind::Epic,
                ListDocumentsOwnerKind::Task => DocumentOwnerKind::Task,
            }),
            owner_id,
            kind: kind.map(|kind| match kind {
                ListDocumentsKind::Brief => DocumentKind::Brief,
                ListDocumentsKind::Plan => DocumentKind::Plan,
                ListDocumentsKind::Finding => DocumentKind::Finding,
                ListDocumentsKind::Handoff => DocumentKind::Handoff,
                ListDocumentsKind::Note => DocumentKind::Note,
                ListDocumentsKind::Decision => DocumentKind::Decision,
            }),
        };
        match store
            .list_documents(
                &ProjectId::from_uuid(project),
                &filters,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, content::document);
                ListDocumentsResponse::Ok(wire::PageDocument { items, next_cursor })
            }
            Err(err) => domain_problem!(ListDocumentsResponse, err),
        }
    }

    async fn create_document(
        &self,
        project_id: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::DocumentCreate,
    ) -> CreateDocumentResponse {
        let store = require_store!(self, CreateDocumentResponse);
        let principal = require_principal!(CreateDocumentResponse);
        let project = parse_path_uuid!(CreateDocumentResponse, project_id, "project_id");
        let key = parse_idempotency_key!(CreateDocumentResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/documents"),
            &body,
        );
        let ctx = command_context(&principal, key, None, store);
        match store
            .create_document(ctx, project, content::core_document_create(body), &hash)
            .await
        {
            Ok(created) => CreateDocumentResponse::Created(content::document(created.into_inner())),
            Err(err) => command_problem!(CreateDocumentResponse, err),
        }
    }

    async fn get_document(&self, project_id: String, document_id: String) -> GetDocumentResponse {
        let store = require_store!(self, GetDocumentResponse);
        require_principal!(GetDocumentResponse);
        let project = parse_path_uuid!(GetDocumentResponse, project_id, "project_id");
        let document = parse_path_uuid!(GetDocumentResponse, document_id, "document_id");
        match store
            .get_document(
                &ProjectId::from_uuid(project),
                &DocumentId::from_uuid(document),
            )
            .await
        {
            Ok(found) => {
                set_etag(found.revision.value());
                GetDocumentResponse::Ok(content::document(found))
            }
            Err(err) => domain_problem!(GetDocumentResponse, err),
        }
    }

    async fn list_document_revisions(
        &self,
        project_id: String,
        document_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> ListDocumentRevisionsResponse {
        let store = require_store!(self, ListDocumentRevisionsResponse);
        require_principal!(ListDocumentRevisionsResponse);
        let project = parse_path_uuid!(ListDocumentRevisionsResponse, project_id, "project_id");
        let document = parse_path_uuid!(ListDocumentRevisionsResponse, document_id, "document_id");
        match store
            .list_document_revisions(
                &ProjectId::from_uuid(project),
                &DocumentId::from_uuid(document),
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, content::document_revision);
                ListDocumentRevisionsResponse::Ok(wire::PageDocumentRevision { items, next_cursor })
            }
            Err(err) => domain_problem!(ListDocumentRevisionsResponse, err),
        }
    }

    async fn create_document_revision(
        &self,
        project_id: String,
        document_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::RevisionCreate,
    ) -> CreateDocumentRevisionResponse {
        let store = require_store!(self, CreateDocumentRevisionResponse);
        let principal = require_principal!(CreateDocumentRevisionResponse);
        let project = parse_path_uuid!(CreateDocumentRevisionResponse, project_id, "project_id");
        let document = parse_path_uuid!(CreateDocumentRevisionResponse, document_id, "document_id");
        let revision = parse_revision!(CreateDocumentRevisionResponse, if_match);
        let key = parse_idempotency_key!(CreateDocumentRevisionResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let hash = request_hash(
            "POST",
            &format!("/api/v1/projects/{project}/documents/{document}/revisions"),
            &body,
        );
        let ctx = command_context(&principal, key, Some(revision), store);
        match store
            .create_document_revision(
                ctx,
                project,
                DocumentId::from_uuid(document),
                content::core_revision_create(body),
                &hash,
            )
            .await
        {
            Ok(created) => CreateDocumentRevisionResponse::Created(content::document_revision(
                created.into_inner(),
            )),
            Err(err) => revision_problem!(CreateDocumentRevisionResponse, err),
        }
    }

    async fn get_document_revision(
        &self,
        project_id: String,
        revision_id: String,
    ) -> GetDocumentRevisionResponse {
        let store = require_store!(self, GetDocumentRevisionResponse);
        require_principal!(GetDocumentRevisionResponse);
        let project = parse_path_uuid!(GetDocumentRevisionResponse, project_id, "project_id");
        let revision = parse_path_uuid!(GetDocumentRevisionResponse, revision_id, "revision_id");
        match store
            .get_document_revision(
                &ProjectId::from_uuid(project),
                &DocumentRevisionId::from_uuid(revision),
            )
            .await
        {
            Ok(found) => {
                // Revisions are immutable; the ETag is the revision number.
                set_etag(found.number);
                GetDocumentRevisionResponse::Ok(content::document_revision(found))
            }
            Err(err) => domain_problem!(GetDocumentRevisionResponse, err),
        }
    }

    async fn select_task_plan(
        &self,
        project_id: String,
        task_id: String,
        if_match: String,
        idempotency_key: String,
        _x_csrf_token: Option<String>,
        body: wire::SelectPlanInput,
    ) -> SelectTaskPlanResponse {
        let store = require_store!(self, SelectTaskPlanResponse);
        let principal = require_principal!(SelectTaskPlanResponse);
        let project = parse_path_uuid!(SelectTaskPlanResponse, project_id, "project_id");
        let task = parse_path_uuid!(SelectTaskPlanResponse, task_id, "task_id");
        let revision = parse_revision!(SelectTaskPlanResponse, if_match);
        let key = parse_idempotency_key!(SelectTaskPlanResponse, idempotency_key);
        let project = ProjectId::from_uuid(project);
        let ctx = command_context(&principal, key, Some(revision), store);
        let outcome = async {
            let result = store
                .select_task_plan(
                    ctx,
                    project,
                    TaskId::from_uuid(task),
                    shepherd_core::model::SelectPlanInput {
                        document_revision_id: body.document_revision_id,
                    },
                )
                .await?;
            task_body(store, project, result.value.id, &principal).await
        }
        .await;
        match outcome {
            Ok(body) => SelectTaskPlanResponse::Ok(body),
            Err(err) => revision_problem!(SelectTaskPlanResponse, err),
        }
    }
}
