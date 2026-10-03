use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

use crate::error::DomainError;
use crate::model::{Claim, ClaimPhase, ClaimStatus, ProjectId, TaskId};
use crate::storage::Store;
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};

use super::{ListParams, Page, decode_after, effective_limit, encode_cursor, split_page};

#[derive(Debug, Clone, Default)]
pub struct ClaimListFilters {
    pub task_id: Option<TaskId>,
    pub status: Option<ClaimStatus>,
    pub phase: Option<ClaimPhase>,
}

fn claims_filter(project: &ProjectId, filters: &ClaimListFilters) -> String {
    let mut parts = vec![format!("project={project}")];
    if let Some(task_id) = &filters.task_id {
        parts.push(format!("task={task_id}"));
    }
    if let Some(status) = &filters.status {
        parts.push(format!("status={}", status.as_str()));
    }
    if let Some(phase) = &filters.phase {
        parts.push(format!("phase={}", phase.as_str()));
    }
    parts.join("&")
}

fn claim_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Claim, DomainError> {
    let id: String = row.try_get("id")?;
    let task_id: String = row.try_get("task_id")?;
    let actor_id: String = row.try_get("actor_id")?;
    let phase: String = row.try_get("phase")?;
    let submission_id: Option<String> = row.try_get("submission_id")?;
    let acquired_at: String = row.try_get("acquired_at")?;
    let expires_at: String = row.try_get("expires_at")?;
    let effective_status: String = row.try_get("effective_status")?;
    let task_revision: i64 = row.try_get("task_revision")?;
    let plan_revision_id: Option<String> = row.try_get("plan_revision_id")?;

    let submission_ids = submission_id
        .map(|id| parse_uuid("claims.submission_id", &id).map(|u| vec![u]))
        .transpose()?
        .unwrap_or_default();

    let plan_revision_ids = plan_revision_id
        .map(|id| parse_uuid("claims.plan_revision_id", &id).map(|u| vec![u]))
        .transpose()?
        .unwrap_or_default();

    Ok(Claim {
        id: parse_uuid("claims.id", &id)?,
        task_id: TaskId::from_uuid(parse_uuid("claims.task_id", &task_id)?),
        actor_id: crate::model::ActorId::from_uuid(parse_uuid("claims.actor_id", &actor_id)?),
        phase: ClaimPhase::parse(&phase).ok_or_else(|| {
            crate::storage::StorageError::Corrupt(format!("claims.phase: {phase}"))
        })?,
        submission_ids,
        acquired_at: parse_ts("claims.acquired_at", &acquired_at)?,
        expires_at: parse_ts("claims.expires_at", &expires_at)?,
        status: ClaimStatus::parse(&effective_status).ok_or_else(|| {
            crate::storage::StorageError::Corrupt(format!("claims.status: {effective_status}"))
        })?,
        task_revision,
        plan_revision_ids,
    })
}

pub(crate) async fn list_claims(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    filters: &ClaimListFilters,
    params: &ListParams,
    now_ts: &str,
) -> Result<Page<Claim>, DomainError> {
    let limit = effective_limit(params)?;
    let filter = claims_filter(project, filters);
    let after = decode_after(params, "listClaims", &filter)?;

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, task_id, actor_id, phase, submission_id, acquired_at, expires_at, \
         CASE WHEN status = 'active' AND expires_at <= ",
    );
    builder
        .push_bind(now_ts.to_string())
        .push(
            " THEN 'expired' ELSE status END AS effective_status, \
               task_revision, plan_revision_id FROM claims WHERE task_id IN \
               (SELECT id FROM tasks WHERE project_id = ",
        )
        .push_bind(project.to_string())
        .push(")");

    if let Some(task_id) = &filters.task_id {
        builder
            .push(" AND task_id = ")
            .push_bind(task_id.to_string());
    }
    if let Some(status) = &filters.status {
        // Compare against the computed effective_status via a subquery / repeated CASE.
        builder
            .push(" AND CASE WHEN status = 'active' AND expires_at <= ")
            .push_bind(now_ts.to_string())
            .push(" THEN 'expired' ELSE status END = ")
            .push_bind(status.as_str());
    }
    if let Some(phase) = &filters.phase {
        builder.push(" AND phase = ").push_bind(phase.as_str());
    }
    if let Some((acquired_at, id)) = &after {
        builder
            .push(" AND (acquired_at < ")
            .push_bind(acquired_at.clone())
            .push(" OR (acquired_at = ")
            .push_bind(acquired_at.clone())
            .push(" AND id < ")
            .push_bind(id.clone())
            .push("))");
    }
    builder
        .push(" ORDER BY acquired_at DESC, id DESC LIMIT ")
        .push_bind(limit + 1);

    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(claim_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(split_page(items, limit, |claim: &Claim| {
        encode_cursor(
            "listClaims",
            &filter,
            &format_ts(&claim.acquired_at),
            &claim.id.to_string(),
        )
    }))
}

impl Store {
    pub async fn list_claims(
        &self,
        project: &ProjectId,
        filters: &ClaimListFilters,
        params: &ListParams,
    ) -> Result<Page<Claim>, DomainError> {
        let now_ts = format_ts(&self.clock().now());
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(crate::storage::StorageError::from)?;
        let page = list_claims(&mut tx, project, filters, params, &now_ts).await?;
        tx.commit()
            .await
            .map_err(crate::storage::StorageError::from)?;
        Ok(page)
    }
}
