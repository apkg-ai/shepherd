use std::collections::HashMap;

use sqlx::{QueryBuilder, Row, Sqlite};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{Actor, Epic, EpicId, ProjectId, Task, TaskId};
use crate::storage::rows::parse_uuid;
use crate::storage::{StorageError, Store};
use crate::workflow::eligibility::{
    Eligibility, evaluate_epic, evaluate_task, load_epic_snapshots, load_task_snapshots,
};

/// Wire view of a task: the resource plus the caller-dependent read-model
/// fields the REST contract requires on every Task body (plan/07).
#[derive(Debug)]
pub struct TaskView {
    pub task: Task,
    pub eligibility: Eligibility,
    pub selected_plan_revision_ids: Vec<Uuid>,
    pub accepted_plan_submission_ids: Vec<Uuid>,
}

#[derive(Debug)]
pub struct EpicView {
    pub epic: Epic,
    pub eligibility: Eligibility,
}

impl Store {
    pub async fn task_view(
        &self,
        project: &ProjectId,
        task: &TaskId,
        actor: &Actor,
    ) -> Result<TaskView, DomainError> {
        let mut views = self.task_views(project, &[*task], actor).await?;
        views.pop().ok_or(DomainError::NotFound)
    }

    /// Batched snapshot + eligibility evaluation preserving input order; any
    /// id missing from the project is NotFound (plan/05: no per-node queries).
    pub async fn task_views(
        &self,
        project: &ProjectId,
        ids: &[TaskId],
        actor: &Actor,
    ) -> Result<Vec<TaskView>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let now = self.clock().now();
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let mut snapshots = load_task_snapshots(&mut tx, project, ids).await?;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, selected_plan_revision_id, accepted_plan_submission_id \
             FROM tasks WHERE id IN (",
        );
        let mut list = builder.separated(", ");
        for id in ids {
            list.push_bind(id.to_string());
        }
        builder.push(")");
        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;

        let mut plan_ids: HashMap<Uuid, (Vec<Uuid>, Vec<Uuid>)> = HashMap::new();
        for row in &rows {
            let id: String = row.try_get("id")?;
            let selected: Option<String> = row.try_get("selected_plan_revision_id")?;
            let accepted: Option<String> = row.try_get("accepted_plan_submission_id")?;
            plan_ids.insert(
                parse_uuid("tasks.id", &id)?,
                (
                    selected
                        .map(|id| {
                            parse_uuid("tasks.selected_plan_revision_id", &id).map(|u| vec![u])
                        })
                        .transpose()?
                        .unwrap_or_default(),
                    accepted
                        .map(|id| {
                            parse_uuid("tasks.accepted_plan_submission_id", &id).map(|u| vec![u])
                        })
                        .transpose()?
                        .unwrap_or_default(),
                ),
            );
        }

        ids.iter()
            .map(|id| {
                let snapshot = snapshots.remove(id).ok_or(DomainError::NotFound)?;
                let eligibility = evaluate_task(&snapshot, actor, now);
                let (selected_plan_revision_ids, accepted_plan_submission_ids) = plan_ids
                    .remove(&id.as_uuid())
                    .ok_or(DomainError::NotFound)?;
                Ok(TaskView {
                    task: snapshot.task,
                    eligibility,
                    selected_plan_revision_ids,
                    accepted_plan_submission_ids,
                })
            })
            .collect()
    }

    pub async fn epic_view(
        &self,
        project: &ProjectId,
        epic: &EpicId,
        actor: &Actor,
    ) -> Result<EpicView, DomainError> {
        let mut views = self.epic_views(project, &[*epic], actor).await?;
        views.pop().ok_or(DomainError::NotFound)
    }

    pub async fn epic_views(
        &self,
        project: &ProjectId,
        ids: &[EpicId],
        actor: &Actor,
    ) -> Result<Vec<EpicView>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let now = self.clock().now();
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let mut snapshots = load_epic_snapshots(&mut tx, project, ids, now).await?;
        tx.commit().await.map_err(StorageError::from)?;
        ids.iter()
            .map(|id| {
                let snapshot = snapshots.remove(id).ok_or(DomainError::NotFound)?;
                let eligibility = evaluate_epic(&snapshot, actor, now);
                Ok(EpicView {
                    epic: snapshot.epic,
                    eligibility,
                })
            })
            .collect()
    }
}
