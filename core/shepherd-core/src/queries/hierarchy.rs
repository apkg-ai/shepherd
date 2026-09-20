use std::collections::HashMap;

use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use super::{
    ListParams, Page, decode_after, effective_limit, encode_cursor, projects_filter,
    push_page_clauses, scoped_filter, split_page,
};
use crate::error::DomainError;
use crate::model::{
    ActorId, Counts, Goal, GoalId, LifecycleRecord, Project, ProjectId, Revision, TaskType,
    TaskTypeId, goal_completed,
};
use crate::storage::rows::{format_ts, parse_flag, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

const PROJECT_COLUMNS: &str = "id, revision, created_at, updated_at, name, description, \
     settings, archived, archive_actor_id, archive_reason, archive_created_at";
const GOAL_COLUMNS: &str = "id, revision, created_at, updated_at, project_id, title, \
     description, archived, archive_actor_id, archive_reason, archive_created_at";
const TASK_TYPE_COLUMNS: &str =
    "id, revision, created_at, updated_at, project_id, key, label, archived, builtin";

fn stored_revision(column: &str, value: i64) -> Result<Revision, DomainError> {
    Revision::from_stored(value)
        .ok_or_else(|| StorageError::Corrupt(format!("{column}: {value}")).into())
}

fn lifecycle_from_row(
    table: &str,
    row: &SqliteRow,
) -> Result<Option<LifecycleRecord>, DomainError> {
    let actor_id: Option<String> = row.try_get("archive_actor_id")?;
    let Some(actor_id) = actor_id else {
        return Ok(None);
    };
    let reason: Option<String> = row.try_get("archive_reason")?;
    let created_at: Option<String> = row.try_get("archive_created_at")?;
    let (Some(reason), Some(created_at)) = (reason, created_at) else {
        return Err(StorageError::Corrupt(format!("{table}: partial archive record")).into());
    };
    Ok(Some(LifecycleRecord {
        actor_id: ActorId::from_uuid(parse_uuid("archive_actor_id", &actor_id)?),
        reason,
        created_at: parse_ts("archive_created_at", &created_at)?,
    }))
}

// Counts are placeholders here; attach_*_counts fills them from the batched GROUP BY.
pub(crate) fn project_from_row(row: &SqliteRow) -> Result<Project, DomainError> {
    let id: String = row.try_get("id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let settings: String = row.try_get("settings")?;
    Ok(Project {
        id: ProjectId::from_uuid(parse_uuid("projects.id", &id)?),
        revision: stored_revision("projects.revision", row.try_get("revision")?)?,
        created_at: parse_ts("projects.created_at", &created_at)?,
        updated_at: parse_ts("projects.updated_at", &updated_at)?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        settings: serde_json::from_str(&settings)
            .map_err(|err| StorageError::Corrupt(format!("projects.settings: {err}")))?,
        archived: parse_flag("projects.archived", row.try_get("archived")?)?,
        epic_counts: Counts::ZERO,
        archive: lifecycle_from_row("projects", row)?,
    })
}

pub(crate) fn goal_from_row(row: &SqliteRow) -> Result<Goal, DomainError> {
    let id: String = row.try_get("id")?;
    let project_id: String = row.try_get("project_id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    Ok(Goal {
        id: GoalId::from_uuid(parse_uuid("goals.id", &id)?),
        revision: stored_revision("goals.revision", row.try_get("revision")?)?,
        created_at: parse_ts("goals.created_at", &created_at)?,
        updated_at: parse_ts("goals.updated_at", &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid("goals.project_id", &project_id)?),
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        archived: parse_flag("goals.archived", row.try_get("archived")?)?,
        epic_counts: Counts::ZERO,
        completed: false,
        archive: lifecycle_from_row("goals", row)?,
    })
}

pub(crate) fn task_type_from_row(row: &SqliteRow) -> Result<TaskType, DomainError> {
    let id: String = row.try_get("id")?;
    let project_id: String = row.try_get("project_id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    Ok(TaskType {
        id: TaskTypeId::from_uuid(parse_uuid("task_types.id", &id)?),
        revision: stored_revision("task_types.revision", row.try_get("revision")?)?,
        created_at: parse_ts("task_types.created_at", &created_at)?,
        updated_at: parse_ts("task_types.updated_at", &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid("task_types.project_id", &project_id)?),
        key: row.try_get("key")?,
        label: row.try_get("label")?,
        archived: parse_flag("task_types.archived", row.try_get("archived")?)?,
        builtin: parse_flag("task_types.builtin", row.try_get("builtin")?)?,
    })
}

// One batched GROUP BY per page (plan/05); never per-node queries.
async fn epic_counts_by(
    conn: &mut SqliteConnection,
    column: &str,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Counts>, DomainError> {
    let mut map: HashMap<Uuid, Counts> = ids.iter().map(|id| (*id, Counts::ZERO)).collect();
    if map.is_empty() {
        return Ok(map);
    }
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT ");
    builder
        .push(column)
        .push(" AS scope, status, COUNT(*) AS n FROM epics WHERE ")
        .push(column)
        .push(" IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id.to_string());
    }
    builder.push(") GROUP BY scope, status");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    for row in rows {
        let scope: String = row.try_get("scope")?;
        let status: String = row.try_get("status")?;
        let n: i64 = row.try_get("n")?;
        let scope = parse_uuid("epics.scope", &scope)?;
        let counts = map
            .get_mut(&scope)
            .ok_or_else(|| StorageError::Corrupt(format!("epics.{column}: {scope}")))?;
        counts.total += n;
        match status.as_str() {
            "done" => counts.done += n,
            "cancelled" => counts.cancelled += n,
            "proposed" | "open" | "active" => {}
            other => {
                return Err(StorageError::Corrupt(format!("epics.status: {other:?}")).into());
            }
        }
        // waived stays 0: epics carry no waivers in v1; waivers are task-level.
    }
    Ok(map)
}

async fn attach_project_counts(
    conn: &mut SqliteConnection,
    projects: &mut [Project],
) -> Result<(), DomainError> {
    let ids: Vec<Uuid> = projects
        .iter()
        .map(|project| project.id.as_uuid())
        .collect();
    let counts = epic_counts_by(conn, "project_id", &ids).await?;
    for project in projects {
        project.epic_counts = counts
            .get(&project.id.as_uuid())
            .copied()
            .unwrap_or(Counts::ZERO);
    }
    Ok(())
}

async fn attach_goal_counts(
    conn: &mut SqliteConnection,
    goals: &mut [Goal],
) -> Result<(), DomainError> {
    let ids: Vec<Uuid> = goals.iter().map(|goal| goal.id.as_uuid()).collect();
    let counts = epic_counts_by(conn, "goal_id", &ids).await?;
    for goal in goals {
        goal.epic_counts = counts
            .get(&goal.id.as_uuid())
            .copied()
            .unwrap_or(Counts::ZERO);
        goal.completed = goal_completed(&goal.epic_counts);
    }
    Ok(())
}

pub(crate) async fn project_exists(
    conn: &mut SqliteConnection,
    id: &ProjectId,
) -> Result<bool, DomainError> {
    Ok(sqlx::query("SELECT 1 FROM projects WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .is_some())
}

// Shared archived-scope guard: NotFound when missing, true when the project scope is frozen.
pub(crate) async fn project_archived(
    conn: &mut SqliteConnection,
    id: &ProjectId,
) -> Result<bool, DomainError> {
    let archived: Option<i64> = sqlx::query_scalar("SELECT archived FROM projects WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(&mut *conn)
        .await?;
    match archived {
        Some(value) => Ok(parse_flag("projects.archived", value)?),
        None => Err(DomainError::NotFound),
    }
}

pub(crate) async fn find_project(
    conn: &mut SqliteConnection,
    id: &ProjectId,
) -> Result<Option<Project>, DomainError> {
    // AssertSqlSafe: interpolates a compile-time column list only.
    let query = AssertSqlSafe(format!(
        "SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"
    ));
    let row = sqlx::query(query)
        .bind(id.to_string())
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut project = project_from_row(&row)?;
    attach_project_counts(conn, std::slice::from_mut(&mut project)).await?;
    Ok(Some(project))
}

// Scoped by project so a cross-project id never leaks another project's goal.
pub(crate) async fn find_goal(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    goal: &GoalId,
) -> Result<Option<Goal>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {GOAL_COLUMNS} FROM goals WHERE id = ?1 AND project_id = ?2"
    ));
    let row = sqlx::query(query)
        .bind(goal.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut goal = goal_from_row(&row)?;
    attach_goal_counts(conn, std::slice::from_mut(&mut goal)).await?;
    Ok(Some(goal))
}

pub(crate) async fn find_task_type(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task_type: &TaskTypeId,
) -> Result<Option<TaskType>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {TASK_TYPE_COLUMNS} FROM task_types WHERE id = ?1 AND project_id = ?2"
    ));
    sqlx::query(query)
        .bind(task_type.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| task_type_from_row(&row))
        .transpose()
}

pub(crate) async fn list_projects(
    conn: &mut SqliteConnection,
    params: &ListParams,
) -> Result<Page<Project>, DomainError> {
    let limit = effective_limit(params)?;
    let filter = projects_filter(params.include_archived);
    let after = decode_after(params, "listProjects", &filter)?;
    let mut builder =
        QueryBuilder::<Sqlite>::new(format!("SELECT {PROJECT_COLUMNS} FROM projects"));
    push_page_clauses(&mut builder, None, params.include_archived, &after, limit);
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(project_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let mut page = split_page(items, limit, |project: &Project| {
        encode_cursor(
            "listProjects",
            &filter,
            &format_ts(&project.created_at),
            &project.id.to_string(),
        )
    });
    attach_project_counts(conn, &mut page.items).await?;
    Ok(page)
}

pub(crate) async fn list_goals(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    params: &ListParams,
) -> Result<Page<Goal>, DomainError> {
    let limit = effective_limit(params)?;
    // Route membership before cursor validity (plan/04): unknown project is 404.
    if !project_exists(conn, project).await? {
        return Err(DomainError::NotFound);
    }
    let filter = scoped_filter(project, params.include_archived);
    let after = decode_after(params, "listGoals", &filter)?;
    let mut builder = QueryBuilder::<Sqlite>::new(format!("SELECT {GOAL_COLUMNS} FROM goals"));
    push_page_clauses(
        &mut builder,
        Some(project),
        params.include_archived,
        &after,
        limit,
    );
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(goal_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let mut page = split_page(items, limit, |goal: &Goal| {
        encode_cursor(
            "listGoals",
            &filter,
            &format_ts(&goal.created_at),
            &goal.id.to_string(),
        )
    });
    attach_goal_counts(conn, &mut page.items).await?;
    Ok(page)
}

pub(crate) async fn list_task_types(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    params: &ListParams,
) -> Result<Page<TaskType>, DomainError> {
    let limit = effective_limit(params)?;
    if !project_exists(conn, project).await? {
        return Err(DomainError::NotFound);
    }
    let filter = scoped_filter(project, params.include_archived);
    let after = decode_after(params, "listTaskTypes", &filter)?;
    let mut builder =
        QueryBuilder::<Sqlite>::new(format!("SELECT {TASK_TYPE_COLUMNS} FROM task_types"));
    push_page_clauses(
        &mut builder,
        Some(project),
        params.include_archived,
        &after,
        limit,
    );
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(task_type_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(split_page(items, limit, |task_type: &TaskType| {
        encode_cursor(
            "listTaskTypes",
            &filter,
            &format_ts(&task_type.created_at),
            &task_type.id.to_string(),
        )
    }))
}

// Public read surface; downstream never touches the pool directly. Each read runs
// in a deferred read transaction so the entity row and its counts share one snapshot.
impl Store {
    pub async fn get_project(&self, id: &ProjectId) -> Result<Project, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = find_project(&mut tx, id).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn get_goal(&self, project: &ProjectId, goal: &GoalId) -> Result<Goal, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = find_goal(&mut tx, project, goal).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn list_projects(&self, params: &ListParams) -> Result<Page<Project>, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let page = list_projects(&mut tx, params).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(page)
    }

    pub async fn list_goals(
        &self,
        project: &ProjectId,
        params: &ListParams,
    ) -> Result<Page<Goal>, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let page = list_goals(&mut tx, project, params).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(page)
    }

    pub async fn list_task_types(
        &self,
        project: &ProjectId,
        params: &ListParams,
    ) -> Result<Page<TaskType>, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let page = list_task_types(&mut tx, project, params).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(page)
    }
}

#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::commands::CommandContext;
    use crate::model::{Actor, ActorKind, Clock, CommandId, GoalCreate, ProjectCreate, TestClock};
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
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
            label: "owner".to_string(),
            revoked: false,
            created_at: clock.now(),
        };
        let inserted = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
            .await
            .unwrap();
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
        }
    }

    fn ctx(f: &Fixture) -> CommandContext {
        CommandContext {
            actor: f.owner.clone(),
            command_id: CommandId::generate(f.clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision: None,
            now: f.clock.now(),
        }
    }

    async fn project(f: &Fixture, name: &str) -> Project {
        f.store
            .create_project(
                ctx(f),
                ProjectCreate {
                    name: name.to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
    }

    async fn goal(f: &Fixture, project: ProjectId, title: &str) -> Goal {
        f.store
            .create_goal(
                ctx(f),
                project,
                GoalCreate {
                    title: title.to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
    }

    async fn seed_epic(f: &Fixture, project: ProjectId, goal: GoalId, status: &str) {
        let now = format_ts(&f.clock.now());
        let cancelled = status == "cancelled";
        sqlx::query(
            "INSERT INTO epics (id, revision, created_at, updated_at, project_id, goal_id, \
             title, description, status, archived, cancellation_actor_id, cancellation_reason, \
             cancellation_created_at) VALUES (?1, 1, ?2, ?2, ?3, ?4, 'epic', '', ?5, 0, ?6, ?7, ?8)",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(&now)
        .bind(project.to_string())
        .bind(goal.to_string())
        .bind(status)
        .bind(cancelled.then(|| f.owner.id.to_string()))
        .bind(cancelled.then_some("not needed"))
        .bind(cancelled.then_some(now.clone()))
        .execute(f.store.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn epic_counts_aggregate_statuses_and_derive_completion() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let mixed = goal(&f, project.id, "mixed").await;
        let done = goal(&f, project.id, "done").await;
        for status in ["done", "done", "cancelled", "open"] {
            seed_epic(&f, project.id, mixed.id, status).await;
        }
        for status in ["done", "done"] {
            seed_epic(&f, project.id, done.id, status).await;
        }

        let mixed_fetched = f.store.get_goal(&project.id, &mixed.id).await.unwrap();
        assert_eq!(
            mixed_fetched.epic_counts,
            Counts {
                total: 4,
                done: 2,
                cancelled: 1,
                waived: 0
            }
        );
        assert!(!mixed_fetched.completed);

        let done_fetched = f.store.get_goal(&project.id, &done.id).await.unwrap();
        assert_eq!(done_fetched.epic_counts.total, 2);
        assert!(done_fetched.completed);

        let project_fetched = f.store.get_project(&project.id).await.unwrap();
        assert_eq!(
            project_fetched.epic_counts,
            Counts {
                total: 6,
                done: 4,
                cancelled: 1,
                waived: 0
            }
        );

        // Lists attach the same batched counts.
        let listed = f
            .store
            .list_goals(&project.id, &ListParams::default())
            .await
            .unwrap();
        let listed_done = listed.items.iter().find(|g| g.id == done.id).unwrap();
        assert!(listed_done.completed);
        let projects = f.store.list_projects(&ListParams::default()).await.unwrap();
        assert_eq!(projects.items[0].epic_counts.total, 6);
    }

    #[tokio::test]
    async fn goal_pages_tiebreak_on_id_and_terminate() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let g1 = goal(&f, project.id, "g1").await;
        let g2 = goal(&f, project.id, "g2").await;
        f.clock.advance(chrono::TimeDelta::milliseconds(3));
        let g3 = goal(&f, project.id, "g3").await;

        let params = |cursor: Option<String>| ListParams {
            limit: Some(2),
            cursor,
            include_archived: false,
        };
        let page1 = f
            .store
            .list_goals(&project.id, &params(None))
            .await
            .unwrap();
        assert_eq!(
            page1.items.iter().map(|g| g.id).collect::<Vec<_>>(),
            vec![g1.id, g2.id]
        );
        let page2 = f
            .store
            .list_goals(&project.id, &params(page1.next_cursor))
            .await
            .unwrap();
        assert_eq!(page2.items[0].id, g3.id);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn cursors_bind_to_endpoint_filters_and_scope() {
        let f = fixture().await;
        let project_a = project(&f, "A").await;
        let project_b = project(&f, "B").await;
        for title in ["g1", "g2", "g3"] {
            goal(&f, project_a.id, title).await;
            f.clock.advance(chrono::TimeDelta::milliseconds(1));
        }
        let cursor = f
            .store
            .list_goals(
                &project_a.id,
                &ListParams {
                    limit: Some(2),
                    cursor: None,
                    include_archived: false,
                },
            )
            .await
            .unwrap()
            .next_cursor
            .unwrap();

        let reuse = |cursor: &str, include_archived: bool| ListParams {
            limit: Some(2),
            cursor: Some(cursor.to_string()),
            include_archived,
        };
        for err in [
            f.store
                .list_goals(&project_a.id, &reuse(&cursor, true))
                .await
                .unwrap_err(),
            f.store
                .list_goals(&project_b.id, &reuse(&cursor, false))
                .await
                .unwrap_err(),
            f.store
                .list_projects(&reuse(&cursor, false))
                .await
                .unwrap_err(),
            f.store
                .list_task_types(&project_a.id, &reuse(&cursor, false))
                .await
                .unwrap_err(),
        ] {
            assert!(matches!(err, DomainError::InvalidCursor(_)));
        }

        let unknown = ProjectId::generate(f.clock.now());
        assert!(matches!(
            f.store
                .list_goals(&unknown, &ListParams::default())
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
        assert!(matches!(
            f.store
                .list_task_types(&unknown, &ListParams::default())
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
        assert!(matches!(
            f.store.get_project(&unknown).await.unwrap_err(),
            DomainError::NotFound
        ));
        let foreign_goal = goal(&f, project_a.id, "g4").await;
        assert!(matches!(
            f.store
                .get_goal(&project_b.id, &foreign_goal.id)
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
    }

    #[tokio::test]
    async fn archived_goals_are_hidden_unless_requested() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let live = goal(&f, project.id, "live").await;
        let archived = goal(&f, project.id, "archived").await;
        sqlx::query(
            "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'shelved', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(archived.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let visible = f
            .store
            .list_goals(&project.id, &ListParams::default())
            .await
            .unwrap();
        assert_eq!(
            visible.items.iter().map(|g| g.id).collect::<Vec<_>>(),
            vec![live.id]
        );

        let all = f
            .store
            .list_goals(
                &project.id,
                &ListParams {
                    include_archived: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let record = all
            .items
            .iter()
            .find(|g| g.id == archived.id)
            .unwrap()
            .archive
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(record.reason, "shelved");
        assert_eq!(record.actor_id, f.owner.id);
    }

    #[tokio::test]
    async fn task_type_lists_page_in_creation_order() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let page1 = f
            .store
            .list_task_types(
                &project.id,
                &ListParams {
                    limit: Some(4),
                    cursor: None,
                    include_archived: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.items.len(), 4);
        let page2 = f
            .store
            .list_task_types(
                &project.id,
                &ListParams {
                    limit: Some(4),
                    cursor: page1.next_cursor,
                    include_archived: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 2);
        assert!(page2.next_cursor.is_none());
        let keys: Vec<&str> = page1
            .items
            .iter()
            .chain(page2.items.iter())
            .map(|t| t.key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "code",
                "research",
                "design",
                "documentation",
                "test",
                "other"
            ]
        );
        assert!(page1.items.iter().all(|t| t.builtin));
    }
}
