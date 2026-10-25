use shepherd_core::model::ProjectId;

use crate::AppState;
use crate::convert::errors::{domain_problem, parse_path_uuid, require_principal, require_store};
use crate::convert::{list_params, page, work};
use crate::generated::server::api::HistoryApi;
use crate::generated::server::errors::ListHistoryResponse;
use crate::generated::types as wire;

#[async_trait::async_trait]
impl HistoryApi for AppState {
    async fn list_history(
        &self,
        project_id: String,
        cursor: Option<String>,
        limit: Option<i32>,
        resource_id: Option<String>,
    ) -> ListHistoryResponse {
        let store = require_store!(self, ListHistoryResponse);
        require_principal!(ListHistoryResponse);
        let project = parse_path_uuid!(ListHistoryResponse, project_id, "project_id");
        let resource_id = match resource_id {
            Some(value) => Some(parse_path_uuid!(ListHistoryResponse, value, "resource_id")),
            None => None,
        };
        match store
            .list_history(
                &ProjectId::from_uuid(project),
                resource_id,
                &list_params(cursor, limit, None),
            )
            .await
        {
            Ok(found) => {
                let (items, next_cursor) = page(found, work::event);
                ListHistoryResponse::Ok(wire::PageEvent { items, next_cursor })
            }
            Err(err) => domain_problem!(ListHistoryResponse, err),
        }
    }
}
