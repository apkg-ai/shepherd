use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use super::{ListParams, Page, decode_after, effective_limit, encode_cursor, split_page};
use crate::error::DomainError;
use crate::model::{
    Actor, Counts, Dependency, DependencyId, DependencyLevel, EntityStatus, EpicId, GoalId,
    ProjectId, Revision, TaskId,
};
use crate::queries::hierarchy::{epic_row, filter_token, goal_row, project_exists};
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};
use crate::workflow::eligibility::{
    evaluate_epic, evaluate_task, load_epic_snapshots, load_task_snapshots,
};

// Contract bounds (plan/05): complete scoped graph or graph_too_large, never partial.
pub const GRAPH_MAX_NODES: i64 = 2_000;
pub const GRAPH_MAX_DEPENDENCIES: i64 = 4_000;

#[derive(Debug)]
pub struct GraphNode {
    pub id: Uuid,
    pub title: String,
    pub status: EntityStatus,
    pub eligibility: crate::workflow::eligibility::Eligibility,
    pub counts: Counts,
}

#[derive(Debug)]
pub struct Graph {
    pub level: DependencyLevel,
    pub scope_id: Uuid,
    pub nodes: Vec<GraphNode>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Default)]
pub struct DependencyListFilters {
    pub level: Option<DependencyLevel>,
    pub scope_id: Option<Uuid>,
}

pub(crate) const DEPENDENCY_COLUMNS: &str =
    "id, revision, created_at, updated_at, project_id, dependent_id, prerequisite_id";

pub(crate) fn dependency_table(level: DependencyLevel) -> &'static str {
    match level {
        DependencyLevel::Epic => "epic_dependencies",
        DependencyLevel::Task => "task_dependencies",
    }
}

// Single source for the level → dependent-parent mapping: every scoped-edge
// query (graph payloads, bounds, listing, cycle check) derives from this pair
// so the scope rule cannot drift between call sites.
pub(crate) fn scope_parent(level: DependencyLevel) -> (&'static str, &'static str) {
    match level {
        DependencyLevel::Epic => ("epics", "goal_id"),
        DependencyLevel::Task => ("tasks", "epic_id"),
    }
}

// `dependent_id IN (SELECT id FROM <parent> WHERE <column> = ?N)` fragment.
pub(crate) fn scoped_dependent_clause(level: DependencyLevel, placeholder: &str) -> String {
    let (parent, column) = scope_parent(level);
    format!("dependent_id IN (SELECT id FROM {parent} WHERE {column} = {placeholder})")
}

pub(crate) fn dependency_from_row(
    level: DependencyLevel,
    row: &SqliteRow,
) -> Result<Dependency, DomainError> {
    let table = dependency_table(level);
    let id: String = row.try_get("id")?;
    let project_id: String = row.try_get("project_id")?;
    let dependent_id: String = row.try_get("dependent_id")?;
    let prerequisite_id: String = row.try_get("prerequisite_id")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let revision: i64 = row.try_get("revision")?;
    Ok(Dependency {
        id: DependencyId::from_uuid(parse_uuid(&format!("{table}.id"), &id)?),
        revision: Revision::from_stored(revision)
            .ok_or_else(|| StorageError::Corrupt(format!("{table}.revision: {revision}")))?,
        created_at: parse_ts(&format!("{table}.created_at"), &created_at)?,
        updated_at: parse_ts(&format!("{table}.updated_at"), &updated_at)?,
        project_id: ProjectId::from_uuid(parse_uuid(&format!("{table}.project_id"), &project_id)?),
        level,
        dependent_id: parse_uuid(&format!("{table}.dependent_id"), &dependent_id)?,
        prerequisite_id: parse_uuid(&format!("{table}.prerequisite_id"), &prerequisite_id)?,
    })
}

// One collection over two tables: probe epic first, then task.
pub(crate) async fn find_dependency(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    dependency: &DependencyId,
) -> Result<Option<Dependency>, DomainError> {
    for level in [DependencyLevel::Epic, DependencyLevel::Task] {
        let query = AssertSqlSafe(format!(
            "SELECT {DEPENDENCY_COLUMNS} FROM {} WHERE id = ?1 AND project_id = ?2",
            dependency_table(level)
        ));
        let row = sqlx::query(query)
            .bind(dependency.to_string())
            .bind(project.to_string())
            .fetch_optional(&mut *conn)
            .await?;
        if let Some(row) = row {
            return Ok(Some(dependency_from_row(level, &row)?));
        }
    }
    Ok(None)
}

// Scoped edge list for graph payloads; scope invariants keep every edge inside
// the dependent's parent, so joining the dependent side is complete.
async fn scoped_dependencies(
    conn: &mut SqliteConnection,
    level: DependencyLevel,
    scope: &Uuid,
) -> Result<Vec<Dependency>, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT {DEPENDENCY_COLUMNS} FROM {} WHERE {} ORDER BY created_at ASC, id ASC",
        dependency_table(level),
        scoped_dependent_clause(level, "?1"),
    ));
    sqlx::query(query)
        .bind(scope.to_string())
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|row| dependency_from_row(level, row))
        .collect()
}

async fn enforce_graph_bounds(
    conn: &mut SqliteConnection,
    level: DependencyLevel,
    scope: &Uuid,
) -> Result<(), DomainError> {
    let (parent, column) = scope_parent(level);
    let node_sql = AssertSqlSafe(format!("SELECT COUNT(*) FROM {parent} WHERE {column} = ?1"));
    let edge_sql = AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM {} WHERE {}",
        dependency_table(level),
        scoped_dependent_clause(level, "?1"),
    ));
    let nodes: i64 = sqlx::query_scalar(node_sql)
        .bind(scope.to_string())
        .fetch_one(&mut *conn)
        .await?;
    let dependencies: i64 = sqlx::query_scalar(edge_sql)
        .bind(scope.to_string())
        .fetch_one(&mut *conn)
        .await?;
    if nodes > GRAPH_MAX_NODES || dependencies > GRAPH_MAX_DEPENDENCIES {
        return Err(DomainError::GraphTooLarge {
            nodes,
            dependencies,
        });
    }
    Ok(())
}

fn dependencies_filter(project: &ProjectId, filters: &DependencyListFilters) -> String {
    format!(
        "project={project}&level={}&scope={}",
        filter_token(filters.level.map(DependencyLevel::as_str)),
        filter_token(filters.scope_id.as_ref()),
    )
}

// One UNION ALL arm per table; the outer query applies the shared keyset order.
fn push_dependency_arm(
    builder: &mut QueryBuilder<Sqlite>,
    level: DependencyLevel,
    project: &ProjectId,
    scope: Option<&Uuid>,
) {
    let (parent, column) = scope_parent(level);
    builder
        .push("SELECT '")
        .push(level.as_str())
        .push("' AS level, ")
        .push(DEPENDENCY_COLUMNS)
        .push(" FROM ")
        .push(dependency_table(level))
        .push(" WHERE project_id = ")
        .push_bind(project.to_string());
    if let Some(scope) = scope {
        builder
            .push(" AND dependent_id IN (SELECT id FROM ")
            .push(parent)
            .push(" WHERE ")
            .push(column)
            .push(" = ")
            .push_bind(scope.to_string())
            .push(")");
    }
}

// Graph reads and the dependency collection. Like the hierarchy read surface,
// authentication is enforced by the server layer; the actor only shapes
// per-node eligibility.
impl Store {
    pub async fn get_goal_graph(
        &self,
        actor: &Actor,
        project: &ProjectId,
        goal: &GoalId,
    ) -> Result<Graph, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        goal_row(&mut tx, project, goal)
            .await?
            .ok_or(DomainError::NotFound)?;
        let scope = goal.as_uuid();
        enforce_graph_bounds(&mut tx, DependencyLevel::Epic, &scope).await?;

        let ids: Vec<EpicId> = sqlx::query_scalar(
            "SELECT id FROM epics WHERE goal_id = ?1 ORDER BY created_at ASC, id ASC",
        )
        .bind(goal.to_string())
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|id: &String| Ok(EpicId::from_uuid(parse_uuid("epics.id", id)?)))
        .collect::<Result<Vec<_>, DomainError>>()?;
        let now = self.clock().now();
        let mut snapshots = load_epic_snapshots(&mut tx, project, &ids, now).await?;
        let mut nodes = Vec::with_capacity(ids.len());
        for id in &ids {
            let snapshot = snapshots
                .remove(id)
                .ok_or_else(|| StorageError::Corrupt(format!("epics.id: {id}")))?;
            let eligibility = evaluate_epic(&snapshot, actor, now);
            nodes.push(GraphNode {
                id: id.as_uuid(),
                title: snapshot.epic.title,
                status: snapshot.epic.status.into(),
                eligibility,
                counts: snapshot.task_counts,
            });
        }
        let dependencies = scoped_dependencies(&mut tx, DependencyLevel::Epic, &scope).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(Graph {
            level: DependencyLevel::Epic,
            scope_id: scope,
            nodes,
            dependencies,
        })
    }

    pub async fn get_epic_graph(
        &self,
        actor: &Actor,
        project: &ProjectId,
        epic: &EpicId,
    ) -> Result<Graph, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        epic_row(&mut tx, project, epic)
            .await?
            .ok_or(DomainError::NotFound)?;
        let scope = epic.as_uuid();
        enforce_graph_bounds(&mut tx, DependencyLevel::Task, &scope).await?;

        let ids: Vec<TaskId> = sqlx::query_scalar(
            "SELECT id FROM tasks WHERE epic_id = ?1 ORDER BY created_at ASC, id ASC",
        )
        .bind(epic.to_string())
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|id: &String| Ok(TaskId::from_uuid(parse_uuid("tasks.id", id)?)))
        .collect::<Result<Vec<_>, DomainError>>()?;
        let now = self.clock().now();
        let mut snapshots = load_task_snapshots(&mut tx, project, &ids).await?;
        let mut nodes = Vec::with_capacity(ids.len());
        for id in &ids {
            let snapshot = snapshots
                .remove(id)
                .ok_or_else(|| StorageError::Corrupt(format!("tasks.id: {id}")))?;
            let eligibility = evaluate_task(&snapshot, actor, now);
            nodes.push(GraphNode {
                id: id.as_uuid(),
                title: snapshot.task.title,
                status: snapshot.task.status.into(),
                eligibility,
                // Tasks are leaves; the contract still requires the field.
                counts: Counts::ZERO,
            });
        }
        let dependencies = scoped_dependencies(&mut tx, DependencyLevel::Task, &scope).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(Graph {
            level: DependencyLevel::Task,
            scope_id: scope,
            nodes,
            dependencies,
        })
    }

    pub async fn list_dependencies(
        &self,
        project: &ProjectId,
        filters: &DependencyListFilters,
        params: &ListParams,
    ) -> Result<Page<Dependency>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown project is 404.
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        let filter = dependencies_filter(project, filters);
        let after = decode_after(params, "listDependencies", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new("SELECT * FROM (");
        match filters.level {
            Some(level) => {
                push_dependency_arm(&mut builder, level, project, filters.scope_id.as_ref());
            }
            None => {
                push_dependency_arm(
                    &mut builder,
                    DependencyLevel::Epic,
                    project,
                    filters.scope_id.as_ref(),
                );
                builder.push(" UNION ALL ");
                push_dependency_arm(
                    &mut builder,
                    DependencyLevel::Task,
                    project,
                    filters.scope_id.as_ref(),
                );
            }
        }
        builder.push(")");
        if let Some((created_at, id)) = &after {
            builder
                .push(" WHERE (created_at > ")
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
        let rows = builder.build().fetch_all(&mut *tx).await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let level: String = row.try_get("level")?;
            let level = DependencyLevel::parse(&level)
                .ok_or_else(|| StorageError::Corrupt(format!("dependencies.level: {level:?}")))?;
            items.push(dependency_from_row(level, row)?);
        }
        tx.commit().await.map_err(StorageError::from)?;
        Ok(split_page(items, limit, |dependency: &Dependency| {
            encode_cursor(
                "listDependencies",
                &filter,
                &format_ts(&dependency.created_at),
                &dependency.id.to_string(),
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::commands::CommandContext;
    use crate::model::Clock;
    use crate::model::{
        ActorId, ActorKind, CommandId, DependencyCreate, EpicCreate, GoalCreate, ProjectCreate,
        TaskCreate, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::{format_ts as ts_text, insert_actor};
    use crate::storage::testing::{store_options, test_clock};
    use crate::workflow::eligibility::GateCode;

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        goal: GoalId,
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
        let project = store
            .create_project(
                ctx(&owner, &clock),
                ProjectCreate {
                    name: "P".to_string(),
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
                    title: "G".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            goal,
        }
    }

    fn ctx(actor: &Actor, clock: &TestClock) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision: None,
            now: clock.now(),
        }
    }

    async fn epic(f: &Fixture, title: &str) -> EpicId {
        f.clock.advance(chrono::TimeDelta::milliseconds(2));
        f.store
            .create_epic(
                ctx(&f.owner, &f.clock),
                f.project,
                f.goal,
                EpicCreate {
                    title: title.to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id
    }

    async fn task(f: &Fixture, epic: EpicId, title: &str) -> TaskId {
        f.clock.advance(chrono::TimeDelta::milliseconds(2));
        f.store
            .create_task(
                ctx(&f.owner, &f.clock),
                f.project,
                epic,
                TaskCreate {
                    title: title.to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id
    }

    #[tokio::test]
    async fn goal_graph_returns_scoped_nodes_edges_counts_and_eligibility() {
        let f = fixture().await;
        let first = epic(&f, "first").await;
        let second = epic(&f, "second").await;
        task(&f, first, "t").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Epic {
                    dependent_id: second,
                    prerequisite_id: first,
                },
            )
            .await
            .unwrap();
        // A second goal's epic stays out of scope.
        let other_goal = f
            .store
            .create_goal(
                ctx(&f.owner, &f.clock),
                f.project,
                GoalCreate {
                    title: "Other".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        f.store
            .create_epic(
                ctx(&f.owner, &f.clock),
                f.project,
                other_goal,
                EpicCreate {
                    title: "foreign".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap();

        let graph = f
            .store
            .get_goal_graph(&f.owner, &f.project, &f.goal)
            .await
            .unwrap();
        assert_eq!(graph.level, DependencyLevel::Epic);
        assert_eq!(graph.scope_id, f.goal.as_uuid());
        assert_eq!(
            graph.nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![first.as_uuid(), second.as_uuid()]
        );
        assert_eq!(graph.nodes[0].counts.total, 1);
        assert_eq!(graph.nodes[0].status, EntityStatus::Open);
        assert!(graph.nodes[0].eligibility.can_execute);
        // The dependent waits on its prerequisite.
        assert!(!graph.nodes[1].eligibility.can_execute);
        assert!(graph.nodes[1].eligibility.can_plan);
        assert_eq!(graph.dependencies.len(), 1);
        assert_eq!(graph.dependencies[0].dependent_id, second.as_uuid());
        assert_eq!(graph.dependencies[0].prerequisite_id, first.as_uuid());
    }

    #[tokio::test]
    async fn epic_graph_returns_task_nodes_with_zero_counts() {
        let f = fixture().await;
        let owner_epic = epic(&f, "E").await;
        let a = task(&f, owner_epic, "a").await;
        let b = task(&f, owner_epic, "b").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Task {
                    dependent_id: b,
                    prerequisite_id: a,
                },
            )
            .await
            .unwrap();
        let graph = f
            .store
            .get_epic_graph(&f.owner, &f.project, &owner_epic)
            .await
            .unwrap();
        assert_eq!(graph.level, DependencyLevel::Task);
        assert_eq!(graph.scope_id, owner_epic.as_uuid());
        assert_eq!(
            graph.nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![a.as_uuid(), b.as_uuid()]
        );
        assert!(graph.nodes.iter().all(|node| node.counts == Counts::ZERO));
        assert_eq!(graph.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn graphs_include_archived_nodes_with_their_gate() {
        let f = fixture().await;
        let live = epic(&f, "live").await;
        let archived = epic(&f, "archived").await;
        // Direct-SQL fixture: no public command can archive until step 006.
        // plan/03 requires terminal before archive, so the epic is completed too.
        sqlx::query(
            "UPDATE epics SET status = 'done', archived = 1, archive_actor_id = ?1, \
             archive_reason = 'shelved', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(ts_text(&f.clock.now()))
        .bind(archived.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let graph = f
            .store
            .get_goal_graph(&f.owner, &f.project, &f.goal)
            .await
            .unwrap();
        assert_eq!(
            graph.nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![live.as_uuid(), archived.as_uuid()]
        );
        let archived_node = &graph.nodes[1];
        assert!(!archived_node.eligibility.can_plan);
        assert_eq!(
            archived_node.eligibility.reasons[0].code,
            GateCode::Archived
        );
        assert!(archived_node.eligibility.allowed_actions.is_empty());
    }

    #[tokio::test]
    async fn graph_too_large_is_all_or_nothing() {
        let f = fixture().await;
        let big = epic(&f, "big").await;
        let now = ts_text(&f.clock.now());
        let mut tx = f.store.pool().begin().await.unwrap();
        // Direct-SQL fixture: the state is ordinary create_task output; raw SQL
        // exists only for scale — 2001 command transactions would dominate the test.
        for _ in 0..(GRAPH_MAX_NODES + 1) {
            sqlx::query(
                "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, \
                 epic_id, title, description, type_key, status, phase, planning_required, \
                 plan_review, work_review, archived, attempt_count) \
                 VALUES (?1, 1, ?2, ?2, ?3, ?4, 'x', '', 'code', 'open', 'execution', 0, \
                 'none', 'none', 0, 0)",
            )
            .bind(TaskId::generate(f.clock.now()).to_string())
            .bind(&now)
            .bind(f.project.to_string())
            .bind(big.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let err = f
            .store
            .get_epic_graph(&f.owner, &f.project, &big)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::GraphTooLarge {
                nodes,
                dependencies: 0
            } if nodes == GRAPH_MAX_NODES + 1
        ));
    }

    #[tokio::test]
    async fn missing_scopes_are_not_found() {
        let f = fixture().await;
        let err = f
            .store
            .get_goal_graph(&f.owner, &f.project, &GoalId::generate(f.clock.now()))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .get_epic_graph(&f.owner, &f.project, &EpicId::generate(f.clock.now()))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .list_dependencies(
                &ProjectId::generate(f.clock.now()),
                &DependencyListFilters::default(),
                &ListParams::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn list_dependencies_pages_and_filters_across_both_tables() {
        let f = fixture().await;
        let first = epic(&f, "first").await;
        let second = epic(&f, "second").await;
        let a = task(&f, first, "a").await;
        let b = task(&f, first, "b").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Epic {
                    dependent_id: second,
                    prerequisite_id: first,
                },
            )
            .await
            .unwrap();
        f.clock.advance(chrono::TimeDelta::milliseconds(2));
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Task {
                    dependent_id: b,
                    prerequisite_id: a,
                },
            )
            .await
            .unwrap();

        let all = f
            .store
            .list_dependencies(
                &f.project,
                &DependencyListFilters::default(),
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            all.items.iter().map(|d| d.level).collect::<Vec<_>>(),
            vec![DependencyLevel::Epic, DependencyLevel::Task]
        );
        assert!(all.next_cursor.is_none());

        let tasks_only = f
            .store
            .list_dependencies(
                &f.project,
                &DependencyListFilters {
                    level: Some(DependencyLevel::Task),
                    scope_id: None,
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(tasks_only.items.len(), 1);
        assert_eq!(tasks_only.items[0].dependent_id, b.as_uuid());

        let scoped = f
            .store
            .list_dependencies(
                &f.project,
                &DependencyListFilters {
                    level: None,
                    scope_id: Some(first.as_uuid()),
                },
                &ListParams::default(),
            )
            .await
            .unwrap();
        // The epic scope matches the task arm's parent only.
        assert_eq!(scoped.items.len(), 1);
        assert_eq!(scoped.items[0].level, DependencyLevel::Task);

        // Keyset paging across the union.
        let page1 = f
            .store
            .list_dependencies(
                &f.project,
                &DependencyListFilters::default(),
                &ListParams {
                    limit: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.items.len(), 1);
        let page2 = f
            .store
            .list_dependencies(
                &f.project,
                &DependencyListFilters::default(),
                &ListParams {
                    limit: Some(1),
                    cursor: page1.next_cursor,
                    include_archived: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 1);
        assert_ne!(page1.items[0].id, page2.items[0].id);
        assert!(page2.next_cursor.is_none());
    }
}
