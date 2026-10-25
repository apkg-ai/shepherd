use sqlx::{QueryBuilder, Sqlite};

use crate::error::DomainError;
use crate::model::{ProjectId, Session, TaskId};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store};

use super::context::session_from_row;
use super::hierarchy::{project_exists, task_exists};
use super::{ListParams, Page, decode_after, effective_limit, encode_cursor, split_page};

fn sessions_filter(project: &ProjectId, task: &TaskId) -> String {
    format!("project={project}&task={task}")
}

impl Store {
    pub async fn list_task_sessions(
        &self,
        project: &ProjectId,
        task: &TaskId,
        params: &ListParams,
    ) -> Result<Page<Session>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04).
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        if !task_exists(&mut tx, project, task).await? {
            return Err(DomainError::NotFound);
        }
        let filter = sessions_filter(project, task);
        let after = decode_after(params, "listTaskSessions", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, task_id, claim_id, actor_id, phase, started_at, ended_at, outcome, \
             summary, failure_reason, document_revision_ids, links \
             FROM sessions WHERE task_id = ",
        );
        builder.push_bind(task.to_string());
        // Sessions list newest first (plan/07), on the same stamp context assembly uses.
        if let Some((ended_at, id)) = &after {
            builder
                .push(" AND (ended_at < ")
                .push_bind(ended_at.clone())
                .push(" OR (ended_at = ")
                .push_bind(ended_at.clone())
                .push(" AND id < ")
                .push_bind(id.clone())
                .push("))");
        }
        builder
            .push(" ORDER BY ended_at DESC, id DESC LIMIT ")
            .push_bind(limit + 1);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(session_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |session: &Session| {
            encode_cursor(
                "listTaskSessions",
                &filter,
                &format_ts(&session.ended_at),
                &session.id.to_string(),
            )
        }))
    }
}
