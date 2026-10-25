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

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use uuid::Uuid;

    use super::*;
    use crate::commands::CommandContext;
    use crate::model::{
        Actor, ActorId, ActorKind, Clock, CommandId, EpicCreate, GoalCreate, ProjectCreate,
        TaskCreate, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::{format_ts, insert_actor};
    use crate::storage::testing::{seed_submission, store_options, test_clock};

    fn ctx(actor: &Actor, clock: &TestClock) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision: None,
            now: clock.now(),
        }
    }

    fn params(limit: Option<u32>, cursor: Option<String>) -> ListParams {
        ListParams {
            limit,
            cursor,
            include_archived: false,
        }
    }

    #[tokio::test]
    async fn task_sessions_page_newest_first_with_scoped_cursor_and_404s() {
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
                ctx(&owner, &clock),
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
                ctx(&owner, &clock),
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
                ctx(&owner, &clock),
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
                        ctx(&owner, &clock),
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
        let (task_a, task_b) = (tasks[0], tasks[1]);
        for _ in 0..3 {
            seed_submission(
                store.pool(),
                task_a,
                owner.id,
                "accepted",
                &format_ts(&clock.now()),
            )
            .await;
            clock.advance(TimeDelta::hours(1));
        }

        let first = store
            .list_task_sessions(&project, &task_a, &params(Some(2), None))
            .await
            .unwrap();
        assert_eq!(first.items.len(), 2);
        assert!(
            first.items[0].ended_at > first.items[1].ended_at,
            "newest first"
        );
        let cursor = first.next_cursor.clone().unwrap();
        let second = store
            .list_task_sessions(&project, &task_a, &params(Some(2), Some(cursor.clone())))
            .await
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert!(second.next_cursor.is_none());
        assert!(second.items[0].ended_at < first.items[1].ended_at);

        // The cursor is bound to its task filter.
        assert!(matches!(
            store
                .list_task_sessions(&project, &task_b, &params(Some(2), Some(cursor)))
                .await,
            Err(DomainError::InvalidCursor(_))
        ));

        // Route membership is 404 before anything else.
        let unknown_project = ProjectId::generate(clock.now());
        assert!(matches!(
            store
                .list_task_sessions(&unknown_project, &task_a, &params(None, None))
                .await,
            Err(DomainError::NotFound)
        ));
        let unknown_task = TaskId::generate(clock.now());
        assert!(matches!(
            store
                .list_task_sessions(&project, &unknown_task, &params(None, None))
                .await,
            Err(DomainError::NotFound)
        ));
    }
}
