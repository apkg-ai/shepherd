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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::commands::CommandContext;
    use crate::error::DomainError;
    use crate::model::{
        Actor, ActorId, ActorKind, Clock, CommandId, DocumentKind, EpicCreate, GoalCreate, GoalId,
        ProjectCreate, TaskCreate, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{
        create_task_document, seed_submission, store_options, test_clock,
    };

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        goal: GoalId,
        epic: EpicId,
        task_a: TaskId,
        task_b: TaskId,
    }

    fn ctx(actor: &Actor, clock: &TestClock, expected_revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision,
            now: clock.now(),
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = Actor {
            id: ActorId::generate(clock.now()),
            kind: ActorKind::Human,
            label: "owner".into(),
            revoked: false,
            created_at: clock.now(),
        };
        let registered = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();
        let project = store
            .create_project(
                ctx(&owner, &clock, None),
                ProjectCreate {
                    name: "P".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let goal = store
            .create_goal(
                ctx(&owner, &clock, None),
                project,
                GoalCreate {
                    title: "G".into(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let epic = store
            .create_epic(
                ctx(&owner, &clock, None),
                project,
                goal,
                EpicCreate {
                    title: "E".into(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let mut tasks = Vec::new();
        for title in ["A", "B"] {
            tasks.push(
                store
                    .create_task(
                        ctx(&owner, &clock, None),
                        project,
                        epic,
                        TaskCreate {
                            title: title.into(),
                            type_key: "code".into(),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap()
                    .value
                    .id,
            );
        }
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            goal,
            epic,
            task_a: tasks[0],
            task_b: tasks[1],
        }
    }

    #[tokio::test]
    async fn task_view_carries_eligibility_and_selected_plan_ids() {
        let f = fixture().await;

        let view = f
            .store
            .task_view(&f.project, &f.task_a, &f.owner)
            .await
            .unwrap();
        assert_eq!(view.task.id, f.task_a);
        assert!(view.selected_plan_revision_ids.is_empty());
        assert!(view.accepted_plan_submission_ids.is_empty());
        assert!(
            view.eligibility.can_plan || view.eligibility.can_execute,
            "a fresh owner task is actionable: {:?}",
            view.eligibility
        );

        // Views surface the stored plan pointers as 0/1-element arrays.
        let document =
            create_task_document(&f.store, &f.owner, f.project, f.task_a, DocumentKind::Plan).await;
        let selected = document.latest_revision_id.as_uuid();
        let accepted: Uuid = seed_submission(
            f.store.pool(),
            f.task_a,
            f.owner.id,
            "accepted",
            "2026-09-14T00:00:00.000Z",
        )
        .await
        .parse()
        .unwrap();
        sqlx::query(
            "UPDATE tasks SET selected_plan_revision_id = ?1, accepted_plan_submission_id = ?2 \
             WHERE id = ?3",
        )
        .bind(selected.to_string())
        .bind(accepted.to_string())
        .bind(f.task_a.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();
        let view = f
            .store
            .task_view(&f.project, &f.task_a, &f.owner)
            .await
            .unwrap();
        assert_eq!(view.selected_plan_revision_ids, vec![selected]);
        assert_eq!(view.accepted_plan_submission_ids, vec![accepted]);

        // Blocking flips eligibility off with a reason.
        f.store
            .block_task(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                f.task_a,
                "blocked for the view test".into(),
            )
            .await
            .unwrap();
        let view = f
            .store
            .task_view(&f.project, &f.task_a, &f.owner)
            .await
            .unwrap();
        assert!(!view.eligibility.can_plan && !view.eligibility.can_execute);
        assert!(!view.eligibility.reasons.is_empty());

        // Batches preserve input order; unknown ids are NotFound.
        let views = f
            .store
            .task_views(&f.project, &[f.task_b, f.task_a], &f.owner)
            .await
            .unwrap();
        assert_eq!(views[0].task.id, f.task_b);
        assert_eq!(views[1].task.id, f.task_a);
        assert!(matches!(
            f.store
                .task_views(
                    &f.project,
                    &[f.task_a, TaskId::generate(f.clock.now())],
                    &f.owner
                )
                .await,
            Err(DomainError::NotFound)
        ));
    }

    #[tokio::test]
    async fn epic_view_matches_graph_node_eligibility() {
        let f = fixture().await;
        let view = f
            .store
            .epic_view(&f.project, &f.epic, &f.owner)
            .await
            .unwrap();
        assert_eq!(view.epic.id, f.epic);

        let graph = f
            .store
            .get_goal_graph(&f.owner, &f.project, &f.goal)
            .await
            .unwrap();
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == f.epic.as_uuid())
            .unwrap();
        assert_eq!(view.eligibility, node.eligibility);

        assert!(matches!(
            f.store
                .epic_view(&f.project, &EpicId::generate(f.clock.now()), &f.owner)
                .await,
            Err(DomainError::NotFound)
        ));
    }
}
