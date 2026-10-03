use super::{
    CommandContext, CommandResult, PendingEvent, append_events, epic_has_active_work, epic_scope,
    live_actor, missing_after_write, require_revision, task_has_active_work, task_scope,
};
use crate::error::DomainError;
use crate::model::{
    Capability, Epic, EpicId, Goal, GoalId, Project, ProjectId, REASON_MAX_CHARS, Revision, Task,
    TaskId, require_capability, validate_required_text,
};
use crate::queries::hierarchy::{
    find_epic, find_goal, find_project, find_task, goal_row, project_archived, project_row,
};
use crate::storage::Store;
use crate::storage::rows::format_ts;
use sqlx::{AssertSqlSafe, SqliteConnection};

// Archive is visibility only (plan/03): owner-only, one-way in v1, requires
// terminal work, never touches dependency rows and never changes counts.

async fn set_archived(
    conn: &mut SqliteConnection,
    table: &'static str,
    id: &str,
    revision: Revision,
    ctx: &CommandContext,
    reason: &str,
) -> Result<(), DomainError> {
    let query = AssertSqlSafe(format!(
        "UPDATE {table} SET revision = ?1, updated_at = ?2, archived = 1, \
         archive_actor_id = ?3, archive_reason = ?4, archive_created_at = ?2 WHERE id = ?5"
    ));
    sqlx::query(query)
        .bind(revision.value())
        .bind(format_ts(&ctx.now))
        .bind(ctx.actor.id.to_string())
        .bind(reason)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

// Goal/project archive requires only terminal contained work; empty is allowed
// (plan/03). Tasks are checked besides epics so fixture-seeded inconsistencies
// cannot slip a live task under an archived scope.
async fn scope_has_nonterminal_work(
    conn: &mut SqliteConnection,
    scope_column: &'static str,
    scope: &str,
) -> Result<bool, DomainError> {
    let epics = AssertSqlSafe(format!(
        "SELECT 1 FROM epics WHERE {scope_column} = ?1 \
         AND status NOT IN ('done', 'cancelled') LIMIT 1"
    ));
    if sqlx::query(epics)
        .bind(scope)
        .fetch_optional(&mut *conn)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    let tasks = AssertSqlSafe(format!(
        "SELECT 1 FROM tasks WHERE epic_id IN (SELECT id FROM epics WHERE {scope_column} = ?1) \
         AND status NOT IN ('done', 'cancelled') LIMIT 1"
    ));
    Ok(sqlx::query(tasks)
        .bind(scope)
        .fetch_optional(&mut *conn)
        .await?
        .is_some())
}

// Same quiet-work rule the epic/task archives enforce: an unexpired active
// claim or pending submission anywhere in scope must be settled first, or the
// one-way archive would freeze it forever.
async fn scope_has_active_work(
    conn: &mut SqliteConnection,
    scope_column: &'static str,
    scope: &str,
    now: &str,
) -> Result<bool, DomainError> {
    let query = AssertSqlSafe(format!(
        "SELECT 1 FROM tasks t \
         WHERE t.epic_id IN (SELECT id FROM epics WHERE {scope_column} = ?1) AND {} LIMIT 1",
        super::active_work_predicate("?2")
    ));
    Ok(sqlx::query(query)
        .bind(scope)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await?
        .is_some())
}

impl Store {
    pub async fn archive_project(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        reason: String,
    ) -> Result<CommandResult<Project>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                if scope_has_nonterminal_work(tx, "project_id", &project.to_string()).await? {
                    return Err(DomainError::InvalidState(
                        "project contains nonterminal work".into(),
                    ));
                }
                if scope_has_active_work(
                    tx,
                    "project_id",
                    &project.to_string(),
                    &format_ts(&ctx.now),
                )
                .await?
                {
                    return Err(DomainError::ActiveWork(
                        "a task in the project has an active claim or pending submission".into(),
                    ));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let next = current.revision.next();
                set_archived(tx, "projects", &project.to_string(), next, &ctx, &reason).await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "archiveProject",
                    &reason,
                    vec![PendingEvent::project(project, next)],
                )
                .await?;
                let updated = find_project(tx, &project)
                    .await?
                    .ok_or_else(|| missing_after_write("project"))?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }

    pub async fn archive_goal(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        goal: GoalId,
        reason: String,
    ) -> Result<CommandResult<Goal>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = goal_row(tx, &project, &goal)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived || project_archived(tx, &project).await? {
                    return Err(DomainError::ArchivedScope);
                }
                if scope_has_nonterminal_work(tx, "goal_id", &goal.to_string()).await? {
                    return Err(DomainError::InvalidState(
                        "goal contains nonterminal work".into(),
                    ));
                }
                if scope_has_active_work(tx, "goal_id", &goal.to_string(), &format_ts(&ctx.now))
                    .await?
                {
                    return Err(DomainError::ActiveWork(
                        "a task in the goal has an active claim or pending submission".into(),
                    ));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let next = current.revision.next();
                set_archived(tx, "goals", &goal.to_string(), next, &ctx, &reason).await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "archiveGoal",
                    &reason,
                    vec![PendingEvent::goal(goal, next)],
                )
                .await?;
                let updated = find_goal(tx, &project, &goal)
                    .await?
                    .ok_or_else(|| missing_after_write("goal"))?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }

    pub async fn archive_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        reason: String,
    ) -> Result<CommandResult<Epic>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let scope = epic_scope(tx, &project, &epic).await?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                if !scope.epic.status.is_terminal() {
                    return Err(DomainError::InvalidState(
                        "epic must be terminal before archive".into(),
                    ));
                }
                // Belt and braces against fixture-seeded inconsistency, like the
                // goal/project descendant checks: a live task must never be
                // frozen under a one-way archive.
                let live_task = sqlx::query(
                    "SELECT 1 FROM tasks WHERE epic_id = ?1 \
                     AND status NOT IN ('done', 'cancelled') LIMIT 1",
                )
                .bind(epic.to_string())
                .fetch_optional(&mut **tx)
                .await?;
                if live_task.is_some() {
                    return Err(DomainError::InvalidState(
                        "epic contains nonterminal work".into(),
                    ));
                }
                if epic_has_active_work(tx, epic, &format_ts(&ctx.now)).await? {
                    return Err(DomainError::ActiveWork(
                        "a descendant task has an active claim or pending submission".into(),
                    ));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let next = scope.epic.revision.next();
                set_archived(tx, "epics", &epic.to_string(), next, &ctx, &reason).await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "archiveEpic",
                    &reason,
                    vec![PendingEvent::epic(epic, next, scope.epic.goal_id)],
                )
                .await?;
                let updated = find_epic(tx, &project, &epic)
                    .await?
                    .ok_or_else(|| missing_after_write("epic"))?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }

    pub async fn archive_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
        reason: String,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if !scope.task.status.is_terminal() {
                    return Err(DomainError::InvalidState(
                        "task must be terminal before archive".into(),
                    ));
                }
                if task_has_active_work(tx, task, &format_ts(&ctx.now)).await? {
                    return Err(DomainError::ActiveWork(
                        "task has an active claim or pending submission".into(),
                    ));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let next = scope.task.revision.next();
                set_archived(tx, "tasks", &task.to_string(), next, &ctx, &reason).await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "archiveTask",
                    &reason,
                    vec![PendingEvent::task(task, next, scope.task.epic_id)],
                )
                .await?;
                let updated = find_task(tx, &project, &task)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::model::{
        Actor, ActorKind, Clock, DependencyCreate, EpicCreate, GoalCreate, ProjectCreate,
        TaskCreate, TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
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

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind: ActorKind::Human,
            label: "owner".to_string(),
            revoked: false,
            created_at: clock.now(),
        };
        let registered = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
        }
    }

    struct Scope {
        project: ProjectId,
        goal: GoalId,
        epic: EpicId,
        task: TaskId,
    }

    async fn scope(f: &Fixture) -> Scope {
        let project = f
            .store
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
            .id;
        let goal = f
            .store
            .create_goal(
                ctx(&f.owner, &f.clock, None),
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
        let epic = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project,
                goal,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let task = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project,
                epic,
                TaskCreate {
                    title: "T".to_string(),
                    description: None,
                    type_key: "code".to_string(),
                    planning_required: None,
                    plan_review: None,
                    work_review: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        Scope {
            project,
            goal,
            epic,
            task,
        }
    }

    async fn seed_pending_submission(f: &Fixture, task: TaskId) -> String {
        crate::storage::testing::seed_submission(
            f.store.pool(),
            task,
            f.owner.id,
            "pending",
            &format_ts(&f.clock.now()),
        )
        .await
    }

    async fn complete_task(f: &Fixture, project: ProjectId, task: TaskId, revision: i64) {
        f.store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(revision)), project, task)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn archive_task_needs_owner_terminal_state_and_quiet_work() {
        let f = fixture().await;
        let s = scope(&f).await;
        let agent = Actor {
            id: crate::model::ActorId::generate(f.clock.now()),
            kind: ActorKind::Agent,
            label: "agent".to_string(),
            revoked: false,
            created_at: f.clock.now(),
        };
        let registered = agent.clone();
        f.store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();

        let err = f
            .store
            .archive_task(
                ctx(&agent, &f.clock, Some(1)),
                s.project,
                s.task,
                "old".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        let err = f
            .store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task,
                "old".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));

        complete_task(&f, s.project, s.task, 1).await;
        let submission = seed_pending_submission(&f, s.task).await;
        let err = f
            .store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task,
                "old".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));

        sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?1")
            .bind(&submission)
            .execute(f.store.pool())
            .await
            .unwrap();
        let err = f
            .store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task,
                "  ".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "reason",
                ..
            }
        ));

        let archived = f
            .store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task,
                "old".into(),
            )
            .await
            .unwrap();
        assert!(archived.value.archived);
        assert_eq!(archived.value.archive.expect("record").reason, "old");
        assert_eq!(archived.value.revision.value(), 3);
        let reason: String = sqlx::query_scalar("SELECT reason FROM events WHERE id = ?1")
            .bind(archived.events[0])
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(reason, "old");

        // Archive is one-way and freezes the scope beneath it.
        let err = f
            .store
            .archive_task(
                ctx(&f.owner, &f.clock, Some(3)),
                s.project,
                s.task,
                "twice".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = f
            .store
            .waive_task(
                ctx(&f.owner, &f.clock, Some(3)),
                s.project,
                s.task,
                "late".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn archive_epic_requires_terminal_epic_and_no_descendant_work() {
        let f = fixture().await;
        let s = scope(&f).await;

        let err = f
            .store
            .archive_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.epic,
                "done".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));

        complete_task(&f, s.project, s.task, 1).await;
        // The cascade completed the epic; a pending submission still blocks.
        let submission = seed_pending_submission(&f, s.task).await;
        let err = f
            .store
            .archive_epic(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.epic,
                "done".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));

        sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?1")
            .bind(&submission)
            .execute(f.store.pool())
            .await
            .unwrap();
        let archived = f
            .store
            .archive_epic(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.epic,
                "done".into(),
            )
            .await
            .unwrap();
        assert!(archived.value.archived);
        // Counts are unaffected by archive (plan/03).
        assert_eq!(archived.value.task_counts.total, 1);
        assert_eq!(archived.value.task_counts.done, 1);

        let err = f
            .store
            .block_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task,
                "beneath".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn goal_and_project_archive_require_only_terminal_work() {
        let f = fixture().await;
        let s = scope(&f).await;

        let err = f
            .store
            .archive_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.goal,
                "x".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
        let err = f
            .store
            .archive_project(ctx(&f.owner, &f.clock, Some(1)), s.project, "x".into())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));

        f.store
            .cancel_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.epic,
                "descoped".into(),
            )
            .await
            .unwrap();
        let archived_goal = f
            .store
            .archive_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.goal,
                "wrapped".into(),
            )
            .await
            .unwrap();
        assert!(archived_goal.value.archived);

        let archived_project = f
            .store
            .archive_project(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                "wrapped".into(),
            )
            .await
            .unwrap();
        assert!(archived_project.value.archived);

        let err = f
            .store
            .archive_goal(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.goal,
                "twice".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn goal_and_project_archive_reject_pending_work_in_scope() {
        let f = fixture().await;
        let s = scope(&f).await;
        f.store
            .cancel_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.epic,
                "descoped".into(),
            )
            .await
            .unwrap();
        // Terminal work, but a pending submission is still unsettled: the
        // one-way archive must not freeze it (same rule as epic/task archive).
        let submission = seed_pending_submission(&f, s.task).await;
        let err = f
            .store
            .archive_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.goal,
                "x".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));
        let err = f
            .store
            .archive_project(ctx(&f.owner, &f.clock, Some(1)), s.project, "x".into())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));

        sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?1")
            .bind(&submission)
            .execute(f.store.pool())
            .await
            .unwrap();
        f.store
            .archive_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.goal,
                "wrapped".into(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn archive_epic_rejects_a_live_task_under_a_terminal_epic() {
        let f = fixture().await;
        let s = scope(&f).await;
        f.store
            .cancel_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.epic,
                "descoped".into(),
            )
            .await
            .unwrap();
        // Fixture-seeded inconsistency: revive the cancelled task directly.
        sqlx::query(
            "UPDATE tasks SET status = 'open', phase = 'execution', cancellation_actor_id = NULL, \
             cancellation_reason = NULL, cancellation_created_at = NULL WHERE id = ?1",
        )
        .bind(s.task.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .archive_epic(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.epic,
                "wrapped".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    #[tokio::test]
    async fn archive_never_removes_dependency_edges() {
        let f = fixture().await;
        let s = scope(&f).await;
        let other = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                s.project,
                s.goal,
                EpicCreate {
                    title: "Other".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                s.project,
                DependencyCreate::Epic {
                    dependent_id: other.id,
                    prerequisite_id: s.epic,
                },
            )
            .await
            .unwrap();
        complete_task(&f, s.project, s.task, 1).await;
        f.store
            .archive_epic(
                ctx(&f.owner, &f.clock, Some(3)),
                s.project,
                s.epic,
                "done".into(),
            )
            .await
            .unwrap();
        let edges: i64 = sqlx::query_scalar("SELECT count(*) FROM epic_dependencies")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(edges, 1);
    }
}
