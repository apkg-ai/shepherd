use sqlx::{Row, SqliteConnection};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    ActorId, ProjectId, Revision, Submission, SubmissionKind, SubmissionStatus, TaskId,
};
use crate::storage::StorageError;
use crate::storage::rows::{parse_ts, parse_uuid};

use super::hierarchy::parse_review_policy;

pub(crate) fn submission_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Submission, DomainError> {
    let id: String = row.try_get("id")?;
    let revision: i64 = row.try_get("revision")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let task_id: String = row.try_get("task_id")?;
    let kind: String = row.try_get("kind")?;
    let producer_id: String = row.try_get("producer_id")?;
    let document_revision_ids: String = row.try_get("document_revision_ids")?;
    let session_id: String = row.try_get("session_id")?;
    let policy: String = row.try_get("policy")?;
    let status: String = row.try_get("status")?;
    let created_context_revision: i64 = row.try_get("created_context_revision")?;

    let stored_ids: Vec<String> = serde_json::from_str(&document_revision_ids).map_err(|err| {
        StorageError::Corrupt(format!("submissions.document_revision_ids: {err}"))
    })?;
    let document_revision_ids = stored_ids
        .iter()
        .map(|id| parse_uuid("submissions.document_revision_ids", id))
        .collect::<Result<Vec<Uuid>, _>>()?;

    Ok(Submission {
        id: parse_uuid("submissions.id", &id)?,
        revision: Revision::from_stored(revision)
            .ok_or_else(|| StorageError::Corrupt(format!("submissions.revision: {revision}")))?,
        created_at: parse_ts("submissions.created_at", &created_at)?,
        updated_at: parse_ts("submissions.updated_at", &updated_at)?,
        task_id: TaskId::from_uuid(parse_uuid("submissions.task_id", &task_id)?),
        kind: SubmissionKind::parse(&kind)
            .ok_or_else(|| StorageError::Corrupt(format!("submissions.kind: {kind}")))?,
        producer_id: ActorId::from_uuid(parse_uuid("submissions.producer_id", &producer_id)?),
        document_revision_ids,
        session_id: parse_uuid("submissions.session_id", &session_id)?,
        policy: parse_review_policy("submissions.policy", &policy)?,
        status: SubmissionStatus::parse(&status)
            .ok_or_else(|| StorageError::Corrupt(format!("submissions.status: {status}")))?,
        created_context_revision,
    })
}

/// Project-scoped submission lookup; None when the id is unknown or foreign.
pub(crate) async fn submission_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    submission_id: &Uuid,
) -> Result<Option<Submission>, DomainError> {
    let row = sqlx::query(
        "SELECT s.id, s.revision, s.created_at, s.updated_at, s.task_id, s.kind, \
         s.producer_id, s.document_revision_ids, s.session_id, s.policy, s.status, \
         s.created_context_revision FROM submissions s \
         JOIN tasks t ON t.id = s.task_id WHERE s.id = ?1 AND t.project_id = ?2",
    )
    .bind(submission_id.to_string())
    .bind(project.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    row.as_ref().map(submission_from_row).transpose()
}
