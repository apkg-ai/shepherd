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
    ActorId, Counts, Epic, EpicId, EpicStatus, Goal, GoalId, LifecycleRecord, Project, ProjectId,
    ReviewPolicy, Revision, Task, TaskId, TaskPhase, TaskStatus, TaskType, TaskTypeId,
    goal_completed,
};
use crate::storage::rows::{format_ts, parse_flag, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

const PROJECT_COLUMNS: &str = "id, revision, created_at, updated_at, name, description, \
     settings, archived, archive_actor_id, archive_reason, archive_created_at";
const GOAL_COLUMNS: &str = "id, revision, created_at, updated_at, project_id, title, \
     description, archived, archive_actor_id, archive_reason, archive_created_at";
const TASK_TYPE_COLUMNS: &str =
    "id, revision, created_at, updated_at, project_id, key, label, archived, builtin";

pub(crate) const EPIC_COLUMNS: &str = "id, revision, created_at, updated_at, project_id, goal_id, \
     title, description, status, archived, \
     block_actor_id, block_reason, block_created_at, \
     archive_actor_id, archive_reason, archive_created_at, \
     cancellation_actor_id, cancellation_reason, cancellation_created_at";

pub(crate) const TASK_COLUMNS: &str = "id, revision, created_at, updated_at, project_id, epic_id, \
     title, description, type_key, status, phase, planning_required, plan_review, work_review, \
     archived, attempt_count, \
     block_actor_id, block_reason, block_created_at, \
     waiver_actor_id, waiver_reason, waiver_created_at, \
     archive_actor_id, archive_reason, archive_created_at, \
     cancellation_actor_id, cancellation_reason, cancellation_created_at";

fn stored_revision(column: &str, value: i64) -> Result<Revision, DomainError> {
    Revision::from_stored(value)
        .ok_or_else(|| StorageError::Corrupt(format!("{column}: {value}")).into())
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
        archive: lifecycle_record("projects", "archive", row)?,
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
        archive: lifecycle_record("goals", "archive", row)?,
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

fn lifecycle_record(
    table: &str,
    prefix: &str,
    row: &SqliteRow,
) -> Result<Option<LifecycleRecord>, DomainError> {
    let actor_col = format!("{prefix}_actor_id");
    let reason_col = format!("{prefix}_reason");
    let created_col = format!("{prefix}_created_at");
    let actor_id: Option<String> = row.try_get(actor_col.as_str())?;
    let Some(actor_id) = actor_id else {
        return Ok(None);
    };
    let reason: Option<String> = row.try_get(reason_col.as_str())?;
    let created_at: Option<String> = row.try_get(created_col.as_str())?;
    let (Some(reason), Some(created_at)) = (reason, created_at) else {
        return Err(StorageError::Corrupt(format!("{table}: partial {prefix} record")).into());
    };
    Ok(Some(LifecycleRecord {
        actor_id: ActorId::from_uuid(parse_uuid(&actor_col, &actor_id)?),
        reason,
        created_at: parse_ts(&created_col, &created_at)?,
    }))
}

pub(crate) fn epic_from_row(row: &SqliteRow) -> Result<Epic, DomainError> {
    let id: String = row.try_get("id")?;
    let project_id: String = row.try_get("project_id")?;
    let goal_id: String = row.try_get("goal_id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let status: String = row.try_get("status")?;
    Ok(Epic {
        id: EpicId::from_uuid(parse_uuid("epics.id", &id)?),
        revision: stored_revision("epics.revision", row.try_get("revision")?)?,
        created_at: parse_ts("epics.created_at", &created_at)?,
        updated_at: parse_ts("epics.updated_at", &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid("epics.project_id", &project_id)?),
        goal_id: GoalId::from_uuid(parse_uuid("epics.goal_id", &goal_id)?),
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        status: EpicStatus::parse(&status)
            .ok_or_else(|| StorageError::Corrupt(format!("epics.status: {status:?}")))?,
        archived: parse_flag("epics.archived", row.try_get("archived")?)?,
        task_counts: Counts::ZERO,
        block: lifecycle_record("epics", "block", row)?,
        archive: lifecycle_record("epics", "archive", row)?,
        cancellation: lifecycle_record("epics", "cancellation", row)?,
    })
}

pub(crate) fn parse_review_policy(column: &str, value: &str) -> Result<ReviewPolicy, DomainError> {
    match value {
        "human" => Ok(ReviewPolicy::Human),
        "agent" => Ok(ReviewPolicy::Agent),
        "none" => Ok(ReviewPolicy::None),
        other => Err(StorageError::Corrupt(format!("{column}: {other:?}")).into()),
    }
}

pub(crate) fn task_from_row(row: &SqliteRow) -> Result<Task, DomainError> {
    let id: String = row.try_get("id")?;
    let project_id: String = row.try_get("project_id")?;
    let epic_id: String = row.try_get("epic_id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let status: String = row.try_get("status")?;
    let phase: String = row.try_get("phase")?;
    let plan_review: String = row.try_get("plan_review")?;
    let work_review: String = row.try_get("work_review")?;
    Ok(Task {
        id: TaskId::from_uuid(parse_uuid("tasks.id", &id)?),
        revision: stored_revision("tasks.revision", row.try_get("revision")?)?,
        created_at: parse_ts("tasks.created_at", &created_at)?,
        updated_at: parse_ts("tasks.updated_at", &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid("tasks.project_id", &project_id)?),
        epic_id: EpicId::from_uuid(parse_uuid("tasks.epic_id", &epic_id)?),
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        type_key: row.try_get("type_key")?,
        status: TaskStatus::parse(&status)
            .ok_or_else(|| StorageError::Corrupt(format!("tasks.status: {status:?}")))?,
        phase: TaskPhase::parse(&phase)
            .ok_or_else(|| StorageError::Corrupt(format!("tasks.phase: {phase:?}")))?,
        planning_required: parse_flag(
            "tasks.planning_required",
            row.try_get("planning_required")?,
        )?,
        plan_review: parse_review_policy("tasks.plan_review", &plan_review)?,
        work_review: parse_review_policy("tasks.work_review", &work_review)?,
        archived: parse_flag("tasks.archived", row.try_get("archived")?)?,
        attempt_count: row.try_get("attempt_count")?,
        block: lifecycle_record("tasks", "block", row)?,
        waiver: lifecycle_record("tasks", "waiver", row)?,
        archive: lifecycle_record("tasks", "archive", row)?,
        cancellation: lifecycle_record("tasks", "cancellation", row)?,
    })
}

// Column parameter for batched count queries. Using an enum instead of &str
// prevents any future caller from passing user-controlled column names.
pub(crate) enum CountScope {
    Project,
    Goal,
    Epic,
}

impl CountScope {
    fn column(&self) -> &'static str {
        match self {
            CountScope::Project => "project_id",
            CountScope::Goal => "goal_id",
            CountScope::Epic => "epic_id",
        }
    }
}

// One batched GROUP BY per page (plan/05); never per-node queries.
async fn epic_counts_by(
    conn: &mut SqliteConnection,
    scope: CountScope,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Counts>, DomainError> {
    let column = scope.column();
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
    let counts = epic_counts_by(conn, CountScope::Project, &ids).await?;
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
    let counts = epic_counts_by(conn, CountScope::Goal, &ids).await?;
    for goal in goals {
        goal.epic_counts = counts
            .get(&goal.id.as_uuid())
            .copied()
            .unwrap_or(Counts::ZERO);
        goal.completed = goal_completed(&goal.epic_counts);
    }
    Ok(())
}

// Single-query task counts with conditional aggregation for waivers.
pub(crate) async fn task_counts_by(
    conn: &mut SqliteConnection,
    scope: CountScope,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Counts>, DomainError> {
    let column = scope.column();
    let mut map: HashMap<Uuid, Counts> = ids.iter().map(|id| (*id, Counts::ZERO)).collect();
    if map.is_empty() {
        return Ok(map);
    }
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT ");
    builder
        .push(column)
        .push(
            " AS scope, status, COUNT(*) AS n, \
             SUM(CASE WHEN waiver_actor_id IS NOT NULL THEN 1 ELSE 0 END) AS waived \
             FROM tasks WHERE ",
        )
        .push(column)
        .push(" IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id.to_string());
    }
    builder.push(") GROUP BY scope, status");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    for row in rows {
        let scope_val: String = row.try_get("scope")?;
        let status: String = row.try_get("status")?;
        let n: i64 = row.try_get("n")?;
        let waived: i64 = row.try_get("waived")?;
        let scope_val = parse_uuid("tasks.scope", &scope_val)?;
        let counts = map
            .get_mut(&scope_val)
            .ok_or_else(|| StorageError::Corrupt(format!("tasks.{column}: {scope_val}")))?;
        counts.total += n;
        match status.as_str() {
            "done" => counts.done += n,
            "cancelled" => {
                counts.cancelled += n;
                counts.waived += waived;
            }
            "proposed" | "open" | "active" => {}
            other => {
                return Err(StorageError::Corrupt(format!("tasks.status: {other:?}")).into());
            }
        }
    }
    Ok(map)
}

async fn attach_epic_task_counts(
    conn: &mut SqliteConnection,
    epics: &mut [Epic],
) -> Result<(), DomainError> {
    let ids: Vec<Uuid> = epics.iter().map(|epic| epic.id.as_uuid()).collect();
    let counts = task_counts_by(conn, CountScope::Epic, &ids).await?;
    for epic in epics {
        epic.task_counts = counts
            .get(&epic.id.as_uuid())
            .copied()
            .unwrap_or(Counts::ZERO);
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

// Row-only selectors for mutation guard loads: no epic-counts GROUP BY on the write path.
pub(crate) async fn project_row(
    conn: &mut SqliteConnection,
    id: &ProjectId,
) -> Result<Option<Project>, DomainError> {
    // AssertSqlSafe: interpolates a compile-time column list only.
    let query = AssertSqlSafe(format!(
        "SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"
    ));
    sqlx::query(query)
        .bind(id.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| project_from_row(&row))
        .transpose()
}

pub(crate) async fn find_project(
    conn: &mut SqliteConnection,
    id: &ProjectId,
) -> Result<Option<Project>, DomainError> {
    let Some(mut project) = project_row(conn, id).await? else {
        return Ok(None);
    };
    attach_project_counts(conn, std::slice::from_mut(&mut project)).await?;
    Ok(Some(project))
}

// Row-only, scoped by project so a cross-project id never leaks another project's goal.
pub(crate) async fn goal_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    goal: &GoalId,
) -> Result<Option<Goal>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {GOAL_COLUMNS} FROM goals WHERE id = ?1 AND project_id = ?2"
    ));
    sqlx::query(query)
        .bind(goal.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| goal_from_row(&row))
        .transpose()
}

pub(crate) async fn find_goal(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    goal: &GoalId,
) -> Result<Option<Goal>, DomainError> {
    let Some(mut goal) = goal_row(conn, project, goal).await? else {
        return Ok(None);
    };
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

pub(crate) async fn epic_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    epic: &EpicId,
) -> Result<Option<Epic>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {EPIC_COLUMNS} FROM epics WHERE id = ?1 AND project_id = ?2"
    ));
    sqlx::query(query)
        .bind(epic.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| epic_from_row(&row))
        .transpose()
}

pub(crate) async fn find_epic(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    epic: &EpicId,
) -> Result<Option<Epic>, DomainError> {
    let Some(mut epic) = epic_row(conn, project, epic).await? else {
        return Ok(None);
    };
    attach_epic_task_counts(conn, std::slice::from_mut(&mut epic)).await?;
    Ok(Some(epic))
}

pub(crate) async fn task_row(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
) -> Result<Option<Task>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1 AND project_id = ?2"
    ));
    sqlx::query(query)
        .bind(task.to_string())
        .bind(project.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| task_from_row(&row))
        .transpose()
}

// Tasks are leaves: no child counts to attach (unlike find_project/find_goal/find_epic).
pub(crate) async fn find_task(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    task: &TaskId,
) -> Result<Option<Task>, DomainError> {
    task_row(conn, project, task).await
}

pub(crate) async fn goal_exists(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    goal: &GoalId,
) -> Result<bool, DomainError> {
    Ok(
        sqlx::query("SELECT 1 FROM goals WHERE id = ?1 AND project_id = ?2")
            .bind(goal.to_string())
            .bind(project.to_string())
            .fetch_optional(&mut *conn)
            .await?
            .is_some(),
    )
}

pub(crate) async fn epic_exists(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    epic: &EpicId,
) -> Result<bool, DomainError> {
    Ok(
        sqlx::query("SELECT 1 FROM epics WHERE id = ?1 AND project_id = ?2")
            .bind(epic.to_string())
            .bind(project.to_string())
            .fetch_optional(&mut *conn)
            .await?
            .is_some(),
    )
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

// Optional filters per the listEpics contract (plan/07); each narrows the SQL WHERE clause
// and is hashed into the cursor fingerprint so cursors cannot cross filter sets.
#[derive(Debug, Clone, Default)]
pub struct EpicListFilters {
    pub goal_id: Option<GoalId>,
    pub status: Option<EpicStatus>,
}

// Optional filters per the listTasks contract (plan/07); same fingerprinting as epics.
#[derive(Debug, Clone, Default)]
pub struct TaskListFilters {
    pub epic_id: Option<EpicId>,
    pub status: Option<TaskStatus>,
    pub phase: Option<TaskPhase>,
    pub type_key: Option<String>,
}

// Absent filters render "none"; present values are prefixed so a value literally
// named "none" (a legal type key) cannot collide with the absent token.
pub(crate) fn filter_token(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "none".to_string(), |v| format!("some:{v}"))
}

fn epics_filter(project: &ProjectId, filters: &EpicListFilters, include_archived: bool) -> String {
    format!(
        "project={project}&goal={}&status={}&include_archived={include_archived}",
        filter_token(filters.goal_id.as_ref()),
        filter_token(filters.status.map(EpicStatus::as_str)),
    )
}

fn tasks_filter(project: &ProjectId, filters: &TaskListFilters, include_archived: bool) -> String {
    format!(
        "project={project}&epic={}&status={}&phase={}&type_key={}&include_archived={include_archived}",
        filter_token(filters.epic_id.as_ref()),
        filter_token(filters.status.map(TaskStatus::as_str)),
        filter_token(filters.phase.map(TaskPhase::as_str)),
        filter_token(filters.type_key.as_deref()),
    )
}

pub(crate) async fn list_epics(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    filters: &EpicListFilters,
    params: &ListParams,
) -> Result<Page<Epic>, DomainError> {
    let limit = effective_limit(params)?;
    // Route membership before cursor validity (plan/04): unknown project or goal is 404.
    if !project_exists(conn, project).await? {
        return Err(DomainError::NotFound);
    }
    if let Some(goal) = filters.goal_id
        && !goal_exists(conn, project, &goal).await?
    {
        return Err(DomainError::NotFound);
    }
    let filter = epics_filter(project, filters, params.include_archived);
    let after = decode_after(params, "listEpics", &filter)?;
    let mut builder = QueryBuilder::<Sqlite>::new(format!("SELECT {EPIC_COLUMNS} FROM epics"));
    builder
        .push(" WHERE project_id = ")
        .push_bind(project.to_string());
    if let Some(goal) = filters.goal_id {
        builder.push(" AND goal_id = ").push_bind(goal.to_string());
    }
    if let Some(status) = filters.status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    if !params.include_archived {
        builder.push(" AND archived = 0");
    }
    if let Some((created_at, id)) = &after {
        builder
            .push(" AND (created_at > ")
            .push_bind(created_at.clone())
            .push(" OR (created_at = ")
            .push_bind(created_at.clone())
            .push(" AND id > ")
            .push_bind(id.clone())
            .push("))");
    }
    builder
        .push(" ORDER BY created_at ASC, id ASC LIMIT ")
        .push_bind(limit + 1);
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(epic_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let mut page = split_page(items, limit, |epic: &Epic| {
        encode_cursor(
            "listEpics",
            &filter,
            &format_ts(&epic.created_at),
            &epic.id.to_string(),
        )
    });
    attach_epic_task_counts(conn, &mut page.items).await?;
    Ok(page)
}

pub(crate) async fn list_tasks(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    filters: &TaskListFilters,
    params: &ListParams,
) -> Result<Page<Task>, DomainError> {
    let limit = effective_limit(params)?;
    // Route membership before cursor validity (plan/04): unknown project or epic is 404.
    if !project_exists(conn, project).await? {
        return Err(DomainError::NotFound);
    }
    if let Some(epic) = filters.epic_id
        && !epic_exists(conn, project, &epic).await?
    {
        return Err(DomainError::NotFound);
    }
    let filter = tasks_filter(project, filters, params.include_archived);
    let after = decode_after(params, "listTasks", &filter)?;
    let mut builder = QueryBuilder::<Sqlite>::new(format!("SELECT {TASK_COLUMNS} FROM tasks"));
    builder
        .push(" WHERE project_id = ")
        .push_bind(project.to_string());
    if let Some(epic) = filters.epic_id {
        builder.push(" AND epic_id = ").push_bind(epic.to_string());
    }
    if let Some(status) = filters.status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    if let Some(phase) = filters.phase {
        builder.push(" AND phase = ").push_bind(phase.as_str());
    }
    if let Some(type_key) = &filters.type_key {
        builder.push(" AND type_key = ").push_bind(type_key.clone());
    }
    if !params.include_archived {
        builder.push(" AND archived = 0");
    }
    if let Some((created_at, id)) = &after {
        builder
            .push(" AND (created_at > ")
            .push_bind(created_at.clone())
            .push(" OR (created_at = ")
            .push_bind(created_at.clone())
            .push(" AND id > ")
            .push_bind(id.clone())
            .push("))");
    }
    builder
        .push(" ORDER BY created_at ASC, id ASC LIMIT ")
        .push_bind(limit + 1);
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(task_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(split_page(items, limit, |task: &Task| {
        encode_cursor(
            "listTasks",
            &filter,
            &format_ts(&task.created_at),
            &task.id.to_string(),
        )
    }))
}

// Public read surface; downstream never touches the pool directly. Each read runs
// in a deferred read transaction so the entity row and its counts share one snapshot.
// Authentication is enforced by the server layer (step 015), not here; these methods
// accept resource IDs only and assume the caller is authorized.
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

    pub async fn get_epic(&self, project: &ProjectId, epic: &EpicId) -> Result<Epic, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = find_epic(&mut tx, project, epic).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn get_task(&self, project: &ProjectId, task: &TaskId) -> Result<Task, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = find_task(&mut tx, project, task).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn list_epics(
        &self,
        project: &ProjectId,
        filters: &EpicListFilters,
        params: &ListParams,
    ) -> Result<Page<Epic>, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let page = list_epics(&mut tx, project, filters, params).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(page)
    }

    pub async fn list_tasks(
        &self,
        project: &ProjectId,
        filters: &TaskListFilters,
        params: &ListParams,
    ) -> Result<Page<Task>, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let page = list_tasks(&mut tx, project, filters, params).await?;
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

    async fn epic(f: &Fixture, project: ProjectId, goal: GoalId, title: &str) -> Epic {
        f.store
            .create_epic(
                ctx(f),
                project,
                goal,
                crate::model::EpicCreate {
                    title: title.to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
    }

    async fn task(f: &Fixture, project: ProjectId, epic: EpicId, title: &str) -> Task {
        f.store
            .create_task(
                ctx(f),
                project,
                epic,
                crate::model::TaskCreate {
                    title: title.to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
    }

    #[tokio::test]
    async fn epic_get_and_list_with_task_counts() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e1 = epic(&f, project.id, goal.id, "e1").await;
        let _t1 = task(&f, project.id, e1.id, "t1").await;
        let _t2 = task(&f, project.id, e1.id, "t2").await;

        let fetched = f.store.get_epic(&project.id, &e1.id).await.unwrap();
        assert_eq!(fetched.task_counts.total, 2);
        assert_eq!(fetched.task_counts.done, 0);

        let page = f
            .store
            .list_epics(
                &project.id,
                &EpicListFilters {
                    goal_id: Some(goal.id),
                    ..Default::default()
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].task_counts.total, 2);
    }

    #[tokio::test]
    async fn task_get_and_list() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t = task(&f, project.id, e.id, "T").await;

        let fetched = f.store.get_task(&project.id, &t.id).await.unwrap();
        assert_eq!(fetched.title, "T");
        assert_eq!(fetched.epic_id, e.id);

        let page = f
            .store
            .list_tasks(
                &project.id,
                &TaskListFilters {
                    epic_id: Some(e.id),
                    ..Default::default()
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, t.id);
    }

    #[tokio::test]
    async fn epic_and_task_reads_scope_by_project() {
        let f = fixture().await;
        let project_a = project(&f, "A").await;
        let project_b = project(&f, "B").await;
        let goal_a = goal(&f, project_a.id, "GA").await;
        let e = epic(&f, project_a.id, goal_a.id, "E").await;
        let t = task(&f, project_a.id, e.id, "T").await;

        assert!(matches!(
            f.store.get_epic(&project_b.id, &e.id).await.unwrap_err(),
            DomainError::NotFound
        ));
        assert!(matches!(
            f.store.get_task(&project_b.id, &t.id).await.unwrap_err(),
            DomainError::NotFound
        ));
        assert!(matches!(
            f.store
                .list_epics(
                    &project_a.id,
                    &EpicListFilters {
                        goal_id: Some(GoalId::generate(f.clock.now())),
                        ..Default::default()
                    },
                    &ListParams::default()
                )
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
        assert!(matches!(
            f.store
                .list_tasks(
                    &project_a.id,
                    &TaskListFilters {
                        epic_id: Some(EpicId::generate(f.clock.now())),
                        ..Default::default()
                    },
                    &ListParams::default()
                )
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
    }

    #[tokio::test]
    async fn task_counts_include_waived_tasks() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let now = format_ts(&f.clock.now());
        let owner_id = f.owner.id.to_string();

        // Seed: 1 done, 1 cancelled, 1 cancelled+waived.
        for (status, cancelled, waiver) in [
            ("done", false, false),
            ("cancelled", true, false),
            ("cancelled", true, true),
        ] {
            let id = crate::model::TaskId::generate(f.clock.now()).to_string();
            sqlx::query(
                "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, \
                 epic_id, title, description, type_key, status, phase, planning_required, \
                 plan_review, work_review, archived, attempt_count, \
                 cancellation_actor_id, cancellation_reason, cancellation_created_at, \
                 waiver_actor_id, waiver_reason, waiver_created_at) \
                 VALUES (?1, 1, ?2, ?2, ?3, ?4, 'x', '', 'code', ?5, 'complete', 0, \
                 'none', 'none', 0, 0, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .bind(&id)
            .bind(&now)
            .bind(project.id.to_string())
            .bind(e.id.to_string())
            .bind(status)
            .bind(cancelled.then_some(&owner_id))
            .bind(cancelled.then_some("reason"))
            .bind(cancelled.then_some(now.clone()))
            .bind(waiver.then_some(&owner_id))
            .bind(waiver.then_some("waived"))
            .bind(waiver.then_some(now.clone()))
            .execute(f.store.pool())
            .await
            .unwrap();
        }

        let fetched = f.store.get_epic(&project.id, &e.id).await.unwrap();
        assert_eq!(fetched.task_counts.total, 3);
        assert_eq!(fetched.task_counts.done, 1);
        assert_eq!(fetched.task_counts.cancelled, 2);
        assert_eq!(fetched.task_counts.waived, 1);
    }

    #[tokio::test]
    async fn epic_pages_tiebreak_on_id_and_terminate() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e1 = epic(&f, project.id, goal.id, "e1").await;
        let e2 = epic(&f, project.id, goal.id, "e2").await;
        f.clock.advance(chrono::TimeDelta::milliseconds(3));
        let e3 = epic(&f, project.id, goal.id, "e3").await;

        let page1 = f
            .store
            .list_epics(
                &project.id,
                &EpicListFilters {
                    goal_id: Some(goal.id),
                    ..Default::default()
                },
                &ListParams {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            page1.items.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![e1.id, e2.id]
        );
        assert!(page1.next_cursor.is_some());

        let page2 = f
            .store
            .list_epics(
                &project.id,
                &EpicListFilters {
                    goal_id: Some(goal.id),
                    ..Default::default()
                },
                &ListParams {
                    limit: Some(2),
                    cursor: page1.next_cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].id, e3.id);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn task_pages_tiebreak_on_id_and_terminate() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t1 = task(&f, project.id, e.id, "t1").await;
        let t2 = task(&f, project.id, e.id, "t2").await;
        f.clock.advance(chrono::TimeDelta::milliseconds(3));
        let t3 = task(&f, project.id, e.id, "t3").await;

        let page1 = f
            .store
            .list_tasks(
                &project.id,
                &TaskListFilters {
                    epic_id: Some(e.id),
                    ..Default::default()
                },
                &ListParams {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            page1.items.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![t1.id, t2.id]
        );

        let page2 = f
            .store
            .list_tasks(
                &project.id,
                &TaskListFilters {
                    epic_id: Some(e.id),
                    ..Default::default()
                },
                &ListParams {
                    limit: Some(2),
                    cursor: page1.next_cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].id, t3.id);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn list_projects_pages_with_cursors() {
        let f = fixture().await;
        let _p1 = project(&f, "P1").await;
        let _p2 = project(&f, "P2").await;
        f.clock.advance(chrono::TimeDelta::milliseconds(3));
        let p3 = project(&f, "P3").await;

        let page1 = f
            .store
            .list_projects(&ListParams {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page1.items.len(), 2);
        assert!(page1.next_cursor.is_some());

        let page2 = f
            .store
            .list_projects(&ListParams {
                limit: Some(2),
                cursor: page1.next_cursor,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].id, p3.id);
        assert!(page2.next_cursor.is_none());
    }
}
