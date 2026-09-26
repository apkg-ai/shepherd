use sqlx::{AssertSqlSafe, SqliteConnection};
use uuid::Uuid;

use super::{
    CommandContext, CommandResult, PendingEvent, append_events, has_active_work, live_actor,
    require_owner, require_revision,
};
use crate::dag;
use crate::error::DomainError;
use crate::model::{
    ActorKind, Dependency, DependencyCreate, DependencyId, DependencyLevel, ProjectId, Revision,
};
use crate::queries::graph::{dependency_table, find_dependency, scoped_dependent_clause};
use crate::queries::hierarchy::{epic_row, goal_row, project_archived, task_row};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store};

fn missing_after_write(what: &'static str) -> DomainError {
    StorageError::Corrupt(format!("{what} missing after write")).into()
}

// Level-normalized endpoint view: everything the shared guards need.
struct Endpoint {
    id: Uuid,
    revision: Revision,
    terminal: bool,
    proposed: bool,
    /// Owning epic (task level) or owning goal (epic level).
    scope: Uuid,
    archived: bool,
}

async fn task_endpoint(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    id: Uuid,
) -> Result<Endpoint, DomainError> {
    let task = task_row(conn, project, &crate::model::TaskId::from_uuid(id))
        .await?
        .ok_or(DomainError::NotFound)?;
    let epic = epic_row(conn, project, &task.epic_id)
        .await?
        .ok_or_else(|| StorageError::Corrupt(format!("tasks.epic_id: {}", task.epic_id)))?;
    let goal = goal_row(conn, project, &epic.goal_id)
        .await?
        .ok_or_else(|| StorageError::Corrupt(format!("epics.goal_id: {}", epic.goal_id)))?;
    Ok(Endpoint {
        id,
        revision: task.revision,
        // The dependent must be live at both levels (plan/04 base); the epic
        // terminal fold only ever gates the dependent — a terminal
        // prerequisite is allowed.
        terminal: task.status.is_terminal() || epic.status.is_terminal(),
        // Accepted means neither the task nor its owning epic is proposed (plan/04).
        proposed: task.status == crate::model::TaskStatus::Proposed
            || epic.status == crate::model::EpicStatus::Proposed,
        scope: task.epic_id.as_uuid(),
        archived: task.archived || epic.archived || goal.archived,
    })
}

async fn epic_endpoint(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    id: Uuid,
) -> Result<Endpoint, DomainError> {
    let epic = epic_row(conn, project, &crate::model::EpicId::from_uuid(id))
        .await?
        .ok_or(DomainError::NotFound)?;
    let goal = goal_row(conn, project, &epic.goal_id)
        .await?
        .ok_or_else(|| StorageError::Corrupt(format!("epics.goal_id: {}", epic.goal_id)))?;
    Ok(Endpoint {
        id,
        revision: epic.revision,
        terminal: epic.status.is_terminal(),
        proposed: epic.status == crate::model::EpicStatus::Proposed,
        scope: epic.goal_id.as_uuid(),
        archived: epic.archived || goal.archived,
    })
}

async fn load_endpoint(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    level: DependencyLevel,
    id: Uuid,
) -> Result<Endpoint, DomainError> {
    match level {
        DependencyLevel::Epic => epic_endpoint(conn, project, id).await,
        DependencyLevel::Task => task_endpoint(conn, project, id).await,
    }
}

// Guards shared by create and delete, in failure-precedence order
// (archived → terminal → active-work), after membership/capability/revision.
async fn guard_mutation(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    level: DependencyLevel,
    dependent: &Endpoint,
    prerequisite: &Endpoint,
    now: &str,
) -> Result<(), DomainError> {
    if project_archived(conn, project).await? || dependent.archived || prerequisite.archived {
        return Err(DomainError::ArchivedScope);
    }
    // Prerequisite terminal is allowed: a done edge is immediately satisfied and
    // a cancelled one stays unmet (plan/04).
    if dependent.terminal {
        return Err(DomainError::TerminalScope);
    }
    if has_active_work(conn, level, dependent.id, now).await? {
        return Err(DomainError::ActiveWork(
            "dependent work has an active claim or pending submission".into(),
        ));
    }
    Ok(())
}

async fn bump_endpoints(
    conn: &mut SqliteConnection,
    level: DependencyLevel,
    ctx: &CommandContext,
    dependent: &Endpoint,
    prerequisite: &Endpoint,
) -> Result<(Revision, Revision), DomainError> {
    let table = match level {
        DependencyLevel::Epic => "epics",
        DependencyLevel::Task => "tasks",
    };
    let dependent_next = dependent.revision.next();
    let prerequisite_next = prerequisite.revision.next();
    for (id, next) in [
        (dependent.id, dependent_next),
        (prerequisite.id, prerequisite_next),
    ] {
        let query = AssertSqlSafe(format!(
            "UPDATE {table} SET revision = ?1, updated_at = ?2 WHERE id = ?3"
        ));
        sqlx::query(query)
            .bind(next.value())
            .bind(format_ts(&ctx.now))
            .bind(id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    Ok((dependent_next, prerequisite_next))
}

fn endpoint_events(
    level: DependencyLevel,
    dependent: &Endpoint,
    dependent_revision: Revision,
    prerequisite: &Endpoint,
    prerequisite_revision: Revision,
) -> Vec<PendingEvent> {
    match level {
        DependencyLevel::Epic => vec![
            PendingEvent::epic(
                crate::model::EpicId::from_uuid(dependent.id),
                dependent_revision,
                crate::model::GoalId::from_uuid(dependent.scope),
            ),
            PendingEvent::epic(
                crate::model::EpicId::from_uuid(prerequisite.id),
                prerequisite_revision,
                crate::model::GoalId::from_uuid(prerequisite.scope),
            ),
        ],
        DependencyLevel::Task => vec![
            PendingEvent::task(
                crate::model::TaskId::from_uuid(dependent.id),
                dependent_revision,
                crate::model::EpicId::from_uuid(dependent.scope),
            ),
            PendingEvent::task(
                crate::model::TaskId::from_uuid(prerequisite.id),
                prerequisite_revision,
                crate::model::EpicId::from_uuid(prerequisite.scope),
            ),
        ],
    }
}

impl Store {
    // No require_owner and no If-Match: both actor kinds add links; creation
    // reloads both ends under the project write lock instead (plan/07, plan/12).
    pub async fn create_dependency(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: DependencyCreate,
    ) -> Result<CommandResult<Dependency>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let level = input.level();
                let (dependent_id, prerequisite_id) = match input {
                    DependencyCreate::Epic {
                        dependent_id,
                        prerequisite_id,
                    } => (dependent_id.as_uuid(), prerequisite_id.as_uuid()),
                    DependencyCreate::Task {
                        dependent_id,
                        prerequisite_id,
                    } => (dependent_id.as_uuid(), prerequisite_id.as_uuid()),
                };
                // Membership first: missing or foreign endpoints are 404 (plan/04).
                let dependent = load_endpoint(tx, &project, level, dependent_id).await?;
                let prerequisite = load_endpoint(tx, &project, level, prerequisite_id).await?;
                let now_text = format_ts(&ctx.now);
                guard_mutation(tx, &project, level, &dependent, &prerequisite, &now_text).await?;
                // Agents may only link accepted dependent work (plan/03).
                if actor.kind == ActorKind::Agent && dependent.proposed {
                    return Err(DomainError::InvalidState(
                        "agent may only add dependencies to accepted work".into(),
                    ));
                }
                if dependent.id == prerequisite.id {
                    return Err(DomainError::Validation {
                        field: "prerequisite_id",
                        message: "dependency cannot reference itself".into(),
                    });
                }
                if dependent.scope != prerequisite.scope {
                    return Err(DomainError::ScopeMismatch(match level {
                        DependencyLevel::Epic => {
                            "epic dependencies must stay inside one goal".into()
                        }
                        DependencyLevel::Task => {
                            "task dependencies must stay inside one epic".into()
                        }
                    }));
                }
                let table = dependency_table(level);
                // Domain error ahead of UNIQUE(dependent_id, prerequisite_id).
                let duplicate_check = AssertSqlSafe(format!(
                    "SELECT 1 FROM {table} WHERE dependent_id = ?1 AND prerequisite_id = ?2"
                ));
                let duplicate = sqlx::query(duplicate_check)
                    .bind(dependent.id.to_string())
                    .bind(prerequisite.id.to_string())
                    .fetch_optional(&mut **tx)
                    .await?;
                if duplicate.is_some() {
                    return Err(DomainError::Validation {
                        field: "prerequisite_id",
                        message: "dependency already exists".into(),
                    });
                }
                // Acyclicity over the scoped edge list, inside the serialized
                // write transaction — this is what makes the cycle race safe.
                let edges_query = AssertSqlSafe(format!(
                    "SELECT dependent_id, prerequisite_id FROM {table} WHERE {}",
                    scoped_dependent_clause(level, "?1"),
                ));
                let edges: Vec<(String, String)> = sqlx::query_as(edges_query)
                    .bind(dependent.scope.to_string())
                    .fetch_all(&mut **tx)
                    .await?;
                let edges = edges
                    .iter()
                    .map(|(from, to)| {
                        Ok((
                            crate::storage::rows::parse_uuid("dependencies.dependent_id", from)?,
                            crate::storage::rows::parse_uuid("dependencies.prerequisite_id", to)?,
                        ))
                    })
                    .collect::<Result<Vec<(Uuid, Uuid)>, DomainError>>()?;
                if dag::would_create_cycle(&edges, dependent.id, prerequisite.id) {
                    return Err(DomainError::DependencyCycle);
                }

                let id = DependencyId::generate(ctx.now);
                let insert = AssertSqlSafe(format!(
                    "INSERT INTO {table} (id, revision, project_id, dependent_id, \
                     prerequisite_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)"
                ));
                sqlx::query(insert)
                    .bind(id.to_string())
                    .bind(Revision::INITIAL.value())
                    .bind(project.to_string())
                    .bind(dependent.id.to_string())
                    .bind(prerequisite.id.to_string())
                    .bind(&now_text)
                    .execute(&mut **tx)
                    .await?;
                // Both endpoints' structural picture changed: bump to invalidate
                // stale edits (plan/03); descendants stay untouched.
                let (dependent_next, prerequisite_next) =
                    bump_endpoints(tx, level, &ctx, &dependent, &prerequisite).await?;
                let mut pending = vec![PendingEvent::dependency(
                    id,
                    Revision::INITIAL,
                    vec![dependent.id, prerequisite.id, dependent.scope],
                )];
                pending.extend(endpoint_events(
                    level,
                    &dependent,
                    dependent_next,
                    &prerequisite,
                    prerequisite_next,
                ));
                let events = append_events(tx, &project, &ctx, "createDependency", pending).await?;
                let created = find_dependency(tx, &project, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("dependency"))?;
                Ok(CommandResult {
                    value: created,
                    events,
                })
            })
        })
        .await
    }

    /// Humans remove links; agents never remove gates (plan/03, plan/12).
    pub async fn delete_dependency(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        dependency: DependencyId,
    ) -> Result<CommandResult<()>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                // Membership precedes capability (plan/04): a foreign link is 404.
                let current = find_dependency(tx, &project, &dependency)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                let level = current.level;
                let dependent = load_endpoint(tx, &project, level, current.dependent_id).await?;
                let prerequisite =
                    load_endpoint(tx, &project, level, current.prerequisite_id).await?;
                let now_text = format_ts(&ctx.now);
                guard_mutation(tx, &project, level, &dependent, &prerequisite, &now_text).await?;

                let delete = AssertSqlSafe(format!(
                    "DELETE FROM {} WHERE id = ?1",
                    dependency_table(level)
                ));
                sqlx::query(delete)
                    .bind(dependency.to_string())
                    .execute(&mut **tx)
                    .await?;
                let (dependent_next, prerequisite_next) =
                    bump_endpoints(tx, level, &ctx, &dependent, &prerequisite).await?;
                let mut pending = vec![PendingEvent::dependency(
                    dependency,
                    current.revision,
                    vec![dependent.id, prerequisite.id, dependent.scope],
                )];
                pending.extend(endpoint_events(
                    level,
                    &dependent,
                    dependent_next,
                    &prerequisite,
                    prerequisite_next,
                ));
                let events = append_events(tx, &project, &ctx, "deleteDependency", pending).await?;
                Ok(CommandResult { value: (), events })
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::model::{
        Actor, ActorKind, Clock, Epic, EpicCreate, EpicId, Goal, GoalCreate, GoalId, Project,
        ProjectCreate, ProjectSettings, Task, TaskCreate, TaskId, TaskStatus, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};
    use uuid::Uuid;

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
    }

    fn person(clock: &TestClock, kind: ActorKind, label: &str) -> Actor {
        Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind,
            label: label.to_string(),
            revoked: false,
            created_at: clock.now(),
        }
    }

    async fn register(store: &Store, actor: &Actor) {
        let inserted = actor.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
            .await
            .unwrap();
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = person(&clock, ActorKind::Human, "owner");
        register(&store, &owner).await;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
        }
    }

    fn ctx(actor: &Actor, clock: &TestClock, expected_revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: crate::model::CommandId::generate(clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision,
            now: clock.now(),
        }
    }

    async fn project(f: &Fixture) -> Project {
        f.store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "P".to_string(),
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
                ctx(&f.owner, &f.clock, None),
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

    async fn epic(f: &Fixture, project: ProjectId, goal: GoalId, title: &str) -> Epic {
        f.store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project,
                goal,
                EpicCreate {
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
                ctx(&f.owner, &f.clock, None),
                project,
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
    }

    fn task_link(dependent: TaskId, prerequisite: TaskId) -> DependencyCreate {
        DependencyCreate::Task {
            dependent_id: dependent,
            prerequisite_id: prerequisite,
        }
    }

    async fn link_tasks(
        f: &Fixture,
        project: ProjectId,
        dependent: TaskId,
        prerequisite: TaskId,
    ) -> Result<CommandResult<Dependency>, DomainError> {
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project,
                task_link(dependent, prerequisite),
            )
            .await
    }

    struct Scope {
        project: Project,
        epic: Epic,
        a: Task,
        b: Task,
    }

    async fn scope(f: &Fixture) -> Scope {
        let project = project(f).await;
        let goal = goal(f, project.id, "G").await;
        let epic = epic(f, project.id, goal.id, "E").await;
        let a = task(f, project.id, epic.id, "a").await;
        let b = task(f, project.id, epic.id, "b").await;
        Scope {
            project,
            epic,
            a,
            b,
        }
    }

    async fn revision_of(f: &Fixture, table: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT revision FROM {table} WHERE id = ?1"
        )))
        .bind(id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap()
    }

    async fn event_count(f: &Fixture) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap()
    }

    // Direct-SQL fixture: no public command can claim until step 009.
    async fn seed_active_claim(f: &Fixture, task: TaskId, expires_at: &str) {
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
             status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'execute', ?4, ?5, \
             'active', 1, 'hash')",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(expires_at)
        .execute(f.store.pool())
        .await
        .unwrap();
    }

    // Direct-SQL fixture: sessions/submissions get commands in steps 010/011.
    async fn seed_pending_submission(f: &Fixture, task: TaskId) {
        let now = format_ts(&f.clock.now());
        let session_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let mut tx = f.store.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, task_id, claim_id, actor_id, phase, started_at, \
             ended_at, outcome, summary, failure_reason, document_revision_ids, links) \
             VALUES (?1, ?2, 'claim', ?3, 'execute', ?4, ?4, 'succeeded', '', '', '[]', '[]')",
        )
        .bind(&session_id)
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO submissions (id, revision, created_at, updated_at, task_id, kind, \
             producer_id, document_revision_ids, session_id, policy, status, \
             created_context_revision) VALUES (?1, 1, ?2, ?2, ?3, 'work', ?4, '[]', ?5, \
             'human', 'pending', 1)",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(&now)
        .bind(task.to_string())
        .bind(f.owner.id.to_string())
        .bind(&session_id)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn create_task_dependency_persists_and_bumps_both_endpoints() {
        let f = fixture().await;
        let s = scope(&f).await;
        let created = link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
        assert_eq!(created.value.level, DependencyLevel::Task);
        assert_eq!(created.value.revision.value(), 1);
        assert_eq!(created.value.dependent_id, s.a.id.as_uuid());
        assert_eq!(created.value.prerequisite_id, s.b.id.as_uuid());
        assert_eq!(created.value.project_id, s.project.id);
        // dependency.changed plus both endpoint task.changed events.
        assert_eq!(created.events.len(), 3);
        assert_eq!(revision_of(&f, "tasks", s.a.id.as_uuid()).await, 2);
        assert_eq!(revision_of(&f, "tasks", s.b.id.as_uuid()).await, 2);

        let (event_type, affected): (String, String) = sqlx::query_as(
            "SELECT type, affected_ids FROM events WHERE type = 'dependency.changed'",
        )
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(event_type, "dependency.changed");
        let affected: Vec<String> = serde_json::from_str(&affected).unwrap();
        assert_eq!(
            affected,
            vec![
                s.a.id.to_string(),
                s.b.id.to_string(),
                s.epic.id.to_string()
            ]
        );
        // Endpoint events name the owning epic (plan/08).
        let endpoint_ids: Vec<String> = sqlx::query_scalar(
            "SELECT affected_ids FROM events WHERE type = 'task.changed' \
             AND command_id = (SELECT command_id FROM events \
                 WHERE type = 'dependency.changed') ORDER BY id",
        )
        .fetch_all(f.store.pool())
        .await
        .unwrap();
        assert_eq!(endpoint_ids.len(), 2);
        for ids in &endpoint_ids {
            let ids: Vec<String> = serde_json::from_str(ids).unwrap();
            assert_eq!(ids, vec![s.epic.id.to_string()]);
        }
    }

    #[tokio::test]
    async fn create_epic_dependency_scopes_events_to_the_goal() {
        let f = fixture().await;
        let project = project(&f).await;
        let g = goal(&f, project.id, "G").await;
        let e1 = epic(&f, project.id, g.id, "e1").await;
        let e2 = epic(&f, project.id, g.id, "e2").await;
        let created = f
            .store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project.id,
                DependencyCreate::Epic {
                    dependent_id: e1.id,
                    prerequisite_id: e2.id,
                },
            )
            .await
            .unwrap();
        assert_eq!(created.value.level, DependencyLevel::Epic);
        assert_eq!(revision_of(&f, "epics", e1.id.as_uuid()).await, 2);
        assert_eq!(revision_of(&f, "epics", e2.id.as_uuid()).await, 2);
        let affected: String =
            sqlx::query_scalar("SELECT affected_ids FROM events WHERE type = 'dependency.changed'")
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        let affected: Vec<String> = serde_json::from_str(&affected).unwrap();
        assert_eq!(affected[2], g.id.to_string());
        // Endpoint events name the owning goal (plan/08).
        let endpoint_ids: Vec<String> = sqlx::query_scalar(
            "SELECT affected_ids FROM events WHERE type = 'epic.changed' \
             AND command_id = (SELECT command_id FROM events \
                 WHERE type = 'dependency.changed') ORDER BY id",
        )
        .fetch_all(f.store.pool())
        .await
        .unwrap();
        assert_eq!(endpoint_ids.len(), 2);
        for ids in &endpoint_ids {
            let ids: Vec<String> = serde_json::from_str(ids).unwrap();
            assert_eq!(ids, vec![g.id.to_string()]);
        }
    }

    #[tokio::test]
    async fn missing_or_foreign_endpoints_are_not_found() {
        let f = fixture().await;
        let s = scope(&f).await;
        let ghost = TaskId::generate(f.clock.now());
        let err = link_tasks(&f, s.project.id, s.a.id, ghost)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        let other = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "Other".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        let err = link_tasks(&f, other.id, s.a.id, s.b.id).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn duplicate_and_self_links_are_validation_errors() {
        let f = fixture().await;
        let s = scope(&f).await;
        link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
        let events_before = event_count(&f).await;
        let err = link_tasks(&f, s.project.id, s.a.id, s.b.id)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "prerequisite_id",
                ..
            }
        ));
        let err = link_tasks(&f, s.project.id, s.a.id, s.a.id)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "prerequisite_id",
                ..
            }
        ));
        // Failed mutations leave revisions and history untouched.
        assert_eq!(revision_of(&f, "tasks", s.a.id.as_uuid()).await, 2);
        assert_eq!(event_count(&f).await, events_before);
    }

    #[tokio::test]
    async fn cycles_are_rejected_inside_the_write_transaction() {
        let f = fixture().await;
        let s = scope(&f).await;
        let c = task(&f, s.project.id, s.epic.id, "c").await;
        link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
        link_tasks(&f, s.project.id, s.b.id, c.id).await.unwrap();
        let err = link_tasks(&f, s.project.id, c.id, s.a.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::DependencyCycle));
        let edges: i64 = sqlx::query_scalar("SELECT count(*) FROM task_dependencies")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(edges, 2);
    }

    #[tokio::test]
    async fn agents_add_links_to_accepted_dependents_only() {
        let f = fixture().await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let project = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "P".to_string(),
                    settings: Some(ProjectSettings {
                        proposal_gate: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        let g = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, g.id, "E").await;
        let accepted = task(&f, project.id, e.id, "accepted").await;
        let proposed = f
            .store
            .create_task(
                ctx(&agent, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "proposed".to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(proposed.status, TaskStatus::Proposed);

        // Proposed dependent: agents rejected, and a proposed prerequisite is fine.
        let err = f
            .store
            .create_dependency(
                ctx(&agent, &f.clock, None),
                project.id,
                task_link(proposed.id, accepted.id),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
        f.store
            .create_dependency(
                ctx(&agent, &f.clock, None),
                project.id,
                task_link(accepted.id, proposed.id),
            )
            .await
            .unwrap();
        // Owners may sequence proposed work.
        let another = task(&f, project.id, e.id, "another").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project.id,
                task_link(proposed.id, another.id),
            )
            .await
            .unwrap();

        // A task under a proposed epic is not accepted work either (plan/04).
        let proposed_epic = f
            .store
            .create_epic(
                ctx(&agent, &f.clock, None),
                project.id,
                g.id,
                EpicCreate {
                    title: "proposed epic".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;
        let under_a = task(&f, project.id, proposed_epic.id, "under a").await;
        let under_b = task(&f, project.id, proposed_epic.id, "under b").await;
        let err = f
            .store
            .create_dependency(
                ctx(&agent, &f.clock, None),
                project.id,
                task_link(under_a.id, under_b.id),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project.id,
                task_link(under_a.id, under_b.id),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn active_work_on_the_dependent_blocks_mutations() {
        let f = fixture().await;
        let s = scope(&f).await;
        let expiry = format_ts(&(f.clock.now() + chrono::TimeDelta::minutes(5)));
        seed_active_claim(&f, s.a.id, &expiry).await;
        let err = link_tasks(&f, s.project.id, s.a.id, s.b.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
        // A claimed prerequisite does not block: the dependent's work is what matters.
        link_tasks(&f, s.project.id, s.b.id, s.a.id).await.unwrap();

        let c = task(&f, s.project.id, s.epic.id, "c").await;
        seed_pending_submission(&f, c.id).await;
        let err = link_tasks(&f, s.project.id, c.id, s.b.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
    }

    #[tokio::test]
    async fn expired_claims_do_not_block_dependency_mutations() {
        let f = fixture().await;
        let s = scope(&f).await;
        let expiry = format_ts(&f.clock.now());
        seed_active_claim(&f, s.a.id, &expiry).await;
        // Expiration is <= now (plan/04): an exactly-now expiry is inactive.
        link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
    }

    #[tokio::test]
    async fn epic_dependency_mutations_check_descendant_tasks() {
        let f = fixture().await;
        let project = project(&f).await;
        let g = goal(&f, project.id, "G").await;
        let e1 = epic(&f, project.id, g.id, "e1").await;
        let e2 = epic(&f, project.id, g.id, "e2").await;
        let busy = task(&f, project.id, e1.id, "busy").await;
        let expiry = format_ts(&(f.clock.now() + chrono::TimeDelta::minutes(5)));
        seed_active_claim(&f, busy.id, &expiry).await;

        let err = f
            .store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project.id,
                DependencyCreate::Epic {
                    dependent_id: e1.id,
                    prerequisite_id: e2.id,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
        // The prerequisite side may be busy.
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                project.id,
                DependencyCreate::Epic {
                    dependent_id: e2.id,
                    prerequisite_id: e1.id,
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn terminal_dependent_blocks_but_done_prerequisite_is_satisfied() {
        let f = fixture().await;
        let s = scope(&f).await;
        // Direct-SQL fixture: completion commands land in step 006.
        sqlx::query("UPDATE tasks SET status = 'done', phase = 'complete' WHERE id = ?1")
            .bind(s.b.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let err = link_tasks(&f, s.project.id, s.b.id, s.a.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
        link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
    }

    #[tokio::test]
    async fn terminal_owning_epic_blocks_the_dependent_task() {
        let f = fixture().await;
        let s = scope(&f).await;
        // Direct-SQL fixture: epic completion lands in step 006.
        sqlx::query("UPDATE epics SET status = 'done' WHERE id = ?1")
            .bind(s.epic.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let err = link_tasks(&f, s.project.id, s.a.id, s.b.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn archived_scope_blocks_dependency_mutations() {
        let f = fixture().await;
        let s = scope(&f).await;
        let created = link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
        // Direct-SQL fixture: no public command can archive until step 006.
        sqlx::query(
            "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'done', archive_created_at = ?2",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .execute(f.store.pool())
        .await
        .unwrap();
        let c_err = link_tasks(&f, s.project.id, s.b.id, s.a.id)
            .await
            .unwrap_err();
        assert!(matches!(c_err, DomainError::ArchivedScope));
        let d_err = f
            .store
            .delete_dependency(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project.id,
                created.value.id,
            )
            .await
            .unwrap_err();
        assert!(matches!(d_err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn delete_requires_owner_and_matching_revision() {
        let f = fixture().await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let s = scope(&f).await;
        let created = link_tasks(&f, s.project.id, s.a.id, s.b.id).await.unwrap();
        let id = created.value.id;
        let events_before = event_count(&f).await;

        let err = f
            .store
            .delete_dependency(ctx(&agent, &f.clock, Some(1)), s.project.id, id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
        let err = f
            .store
            .delete_dependency(ctx(&f.owner, &f.clock, None), s.project.id, id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PreconditionRequired));
        let err = f
            .store
            .delete_dependency(ctx(&f.owner, &f.clock, Some(9)), s.project.id, id)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::RevisionConflict {
                expected: 9,
                actual: 1
            }
        ));
        let err = f
            .store
            .delete_dependency(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project.id,
                DependencyId::generate(f.clock.now()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        assert_eq!(event_count(&f).await, events_before);
        assert_eq!(revision_of(&f, "tasks", s.a.id.as_uuid()).await, 2);

        let deleted = f
            .store
            .delete_dependency(ctx(&f.owner, &f.clock, Some(1)), s.project.id, id)
            .await
            .unwrap();
        assert_eq!(deleted.events.len(), 3);
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM task_dependencies")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(revision_of(&f, "tasks", s.a.id.as_uuid()).await, 3);
        assert_eq!(revision_of(&f, "tasks", s.b.id.as_uuid()).await, 3);
    }

    #[tokio::test]
    async fn create_ignores_expected_revision_like_other_creates() {
        let f = fixture().await;
        let s = scope(&f).await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, Some(99)),
                s.project.id,
                task_link(s.a.id, s.b.id),
            )
            .await
            .unwrap();
    }
}
