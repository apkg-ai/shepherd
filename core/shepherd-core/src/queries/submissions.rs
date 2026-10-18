use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    ActorId, ProjectId, Review, ReviewDecision, ReviewPolicy, Revision, Submission, SubmissionKind,
    SubmissionStatus, TaskId,
};
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

use super::hierarchy::{parse_review_policy, project_exists, task_exists};
use super::{
    ListParams, Page, decode_after, effective_limit, encode_cursor, push_keyset_tail, split_page,
};

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

#[derive(Debug, Clone, Default)]
pub struct SubmissionListFilters {
    pub task_id: Option<TaskId>,
    pub status: Option<SubmissionStatus>,
    pub kind: Option<SubmissionKind>,
    pub policy: Option<ReviewPolicy>,
}

fn submissions_filter(project: &ProjectId, filters: &SubmissionListFilters) -> String {
    let mut parts = vec![format!("project={project}")];
    if let Some(task_id) = &filters.task_id {
        parts.push(format!("task={task_id}"));
    }
    if let Some(status) = &filters.status {
        parts.push(format!("status={}", status.as_str()));
    }
    if let Some(kind) = &filters.kind {
        parts.push(format!("kind={}", kind.as_str()));
    }
    if let Some(policy) = &filters.policy {
        parts.push(format!("policy={}", policy.as_str()));
    }
    parts.join("&")
}

fn review_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Review, DomainError> {
    let id: String = row.try_get("id")?;
    let submission_id: String = row.try_get("submission_id")?;
    let actor_id: String = row.try_get("actor_id")?;
    let decision: String = row.try_get("decision")?;
    let reason: String = row.try_get("reason")?;
    let created_at: String = row.try_get("created_at")?;
    Ok(Review {
        id: parse_uuid("reviews.id", &id)?,
        submission_id: parse_uuid("reviews.submission_id", &submission_id)?,
        actor_id: ActorId::from_uuid(parse_uuid("reviews.actor_id", &actor_id)?),
        decision: ReviewDecision::parse(&decision)
            .ok_or_else(|| StorageError::Corrupt(format!("reviews.decision: {decision}")))?,
        reason,
        created_at: parse_ts("reviews.created_at", &created_at)?,
    })
}

async fn submission_exists(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    submission: &Uuid,
) -> Result<bool, DomainError> {
    Ok(sqlx::query(
        "SELECT 1 FROM submissions s JOIN tasks t ON t.id = s.task_id \
         WHERE s.id = ?1 AND t.project_id = ?2",
    )
    .bind(submission.to_string())
    .bind(project.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .is_some())
}

impl Store {
    pub async fn list_submissions(
        &self,
        project: &ProjectId,
        filters: &SubmissionListFilters,
        params: &ListParams,
    ) -> Result<Page<Submission>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown project
        // or task-filter target is 404.
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        if let Some(task_id) = &filters.task_id
            && !task_exists(&mut tx, project, task_id).await?
        {
            return Err(DomainError::NotFound);
        }
        let filter = submissions_filter(project, filters);
        let after = decode_after(params, "listSubmissions", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT s.id, s.revision, s.created_at, s.updated_at, s.task_id, s.kind, \
             s.producer_id, s.document_revision_ids, s.session_id, s.policy, s.status, \
             s.created_context_revision FROM submissions s WHERE s.task_id IN \
             (SELECT id FROM tasks WHERE project_id = ",
        );
        builder.push_bind(project.to_string()).push(")");
        if let Some(task_id) = &filters.task_id {
            builder
                .push(" AND s.task_id = ")
                .push_bind(task_id.to_string());
        }
        if let Some(status) = &filters.status {
            builder.push(" AND s.status = ").push_bind(status.as_str());
        }
        if let Some(kind) = &filters.kind {
            builder.push(" AND s.kind = ").push_bind(kind.as_str());
        }
        if let Some(policy) = &filters.policy {
            builder.push(" AND s.policy = ").push_bind(policy.as_str());
        }
        // Submissions list newest first (plan/07).
        if let Some((created_at, id)) = &after {
            builder
                .push(" AND (s.created_at < ")
                .push_bind(created_at.clone())
                .push(" OR (s.created_at = ")
                .push_bind(created_at.clone())
                .push(" AND s.id < ")
                .push_bind(id.clone())
                .push("))");
        }
        builder
            .push(" ORDER BY s.created_at DESC, s.id DESC LIMIT ")
            .push_bind(limit + 1);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(submission_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |submission: &Submission| {
            encode_cursor(
                "listSubmissions",
                &filter,
                &format_ts(&submission.created_at),
                &submission.id.to_string(),
            )
        }))
    }

    pub async fn get_submission(
        &self,
        project: &ProjectId,
        submission: &Uuid,
    ) -> Result<Submission, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let row = submission_row(&mut tx, project, submission).await?;
        tx.commit().await.map_err(StorageError::from)?;
        row.ok_or(DomainError::NotFound)
    }

    pub async fn list_submission_reviews(
        &self,
        project: &ProjectId,
        submission: &Uuid,
        params: &ListParams,
    ) -> Result<Page<Review>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        if !submission_exists(&mut tx, project, submission).await? {
            return Err(DomainError::NotFound);
        }
        let filter = format!("project={project}&submission={submission}");
        let after = decode_after(params, "listSubmissionReviews", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, submission_id, actor_id, decision, reason, created_at \
             FROM reviews WHERE submission_id = ",
        );
        builder.push_bind(submission.to_string());
        push_keyset_tail(&mut builder, " AND ", &after, limit);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(review_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |review: &Review| {
            encode_cursor(
                "listSubmissionReviews",
                &filter,
                &format_ts(&review.created_at),
                &review.id.to_string(),
            )
        }))
    }
}
