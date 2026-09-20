use super::{
    CommandContext, CommandResult, PendingEvent, append_events, live_actor, require_owner,
    require_revision,
};
use crate::error::DomainError;
use crate::model::{
    BUILTIN_TASK_TYPES, DESCRIPTION_MAX_CHARS, Goal, GoalCreate, GoalId, NAME_MAX_CHARS, Project,
    ProjectCreate, ProjectId, ProjectPatch, ProjectSettings, Revision, TaskType, TaskTypeCreate,
    TaskTypeId, TaskTypePatch, TextPatch, validate_long_text, validate_required_text,
    validate_type_key, validate_type_label,
};
use crate::queries::hierarchy::{find_goal, find_project, find_task_type};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store};

fn settings_json(settings: &ProjectSettings) -> Result<String, DomainError> {
    serde_json::to_string(settings)
        .map_err(|err| StorageError::Corrupt(format!("settings: {err}")).into())
}

fn missing_after_write(what: &'static str) -> DomainError {
    StorageError::Corrupt(format!("{what} missing after write")).into()
}

impl Store {
    pub async fn create_project(
        &self,
        ctx: CommandContext,
        input: ProjectCreate,
    ) -> Result<CommandResult<Project>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                require_owner(&actor)?;
                let name = validate_required_text("name", &input.name, NAME_MAX_CHARS)?;
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                let settings = input.settings.unwrap_or_default();
                let id = ProjectId::generate(ctx.now);
                let now_text = format_ts(&ctx.now);
                sqlx::query(
                    "INSERT INTO projects (id, revision, created_at, updated_at, name, \
                     description, settings, archived) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(&now_text)
                .bind(&name)
                .bind(&description)
                .bind(settings_json(&settings)?)
                .execute(&mut **tx)
                .await?;

                // Every project starts with the six built-in registry entries (plan/00).
                let mut pending = vec![PendingEvent::project(id, Revision::INITIAL)];
                for (key, label) in BUILTIN_TASK_TYPES {
                    let type_id = TaskTypeId::generate(ctx.now);
                    sqlx::query(
                        "INSERT INTO task_types (id, revision, created_at, updated_at, \
                         project_id, key, label, archived, builtin) \
                         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0, 1)",
                    )
                    .bind(type_id.to_string())
                    .bind(Revision::INITIAL.value())
                    .bind(&now_text)
                    .bind(id.to_string())
                    .bind(key)
                    .bind(label)
                    .execute(&mut **tx)
                    .await?;
                    pending.push(PendingEvent::task_type(type_id, Revision::INITIAL));
                }

                let events = append_events(tx, &id, &ctx, "createProject", pending).await?;
                let project = find_project(tx, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("project"))?;
                Ok(CommandResult {
                    value: project,
                    events,
                })
            })
        })
        .await
    }

    pub async fn update_project(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        patch: ProjectPatch,
    ) -> Result<CommandResult<Project>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = find_project(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let name = match &patch.name {
                    Some(value) => validate_required_text("name", value, NAME_MAX_CHARS)?,
                    None => current.name,
                };
                let description = match patch.description {
                    Some(value) => {
                        validate_long_text("description", &value, DESCRIPTION_MAX_CHARS)?;
                        value
                    }
                    None => current.description,
                };
                let settings = patch.settings.unwrap_or(current.settings);
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE projects SET revision = ?1, updated_at = ?2, name = ?3, \
                     description = ?4, settings = ?5 WHERE id = ?6",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(&name)
                .bind(&description)
                .bind(settings_json(&settings)?)
                .bind(project.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "updateProject",
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

    pub async fn create_goal(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: GoalCreate,
    ) -> Result<CommandResult<Goal>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let parent = find_project(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                if parent.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let title = validate_required_text("title", &input.title, NAME_MAX_CHARS)?;
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                let id = GoalId::generate(ctx.now);
                sqlx::query(
                    "INSERT INTO goals (id, revision, created_at, updated_at, project_id, \
                     title, description, archived) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(format_ts(&ctx.now))
                .bind(project.to_string())
                .bind(&title)
                .bind(&description)
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "createGoal",
                    vec![PendingEvent::goal(id, Revision::INITIAL)],
                )
                .await?;
                let goal = find_goal(tx, &project, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("goal"))?;
                Ok(CommandResult {
                    value: goal,
                    events,
                })
            })
        })
        .await
    }

    pub async fn update_goal(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        goal: GoalId,
        patch: TextPatch,
    ) -> Result<CommandResult<Goal>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                // Membership precedes the revision check: a foreign goal is 404, never 412.
                let current = find_goal(tx, &project, &goal)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let title = match &patch.title {
                    Some(value) => validate_required_text("title", value, NAME_MAX_CHARS)?,
                    None => current.title,
                };
                let description = match patch.description {
                    Some(value) => {
                        validate_long_text("description", &value, DESCRIPTION_MAX_CHARS)?;
                        value
                    }
                    None => current.description,
                };
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE goals SET revision = ?1, updated_at = ?2, title = ?3, \
                     description = ?4 WHERE id = ?5",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(&title)
                .bind(&description)
                .bind(goal.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "updateGoal",
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

    pub async fn create_task_type(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: TaskTypeCreate,
    ) -> Result<CommandResult<TaskType>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let parent = find_project(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                if parent.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let key = validate_type_key(&input.key)?;
                let label = validate_type_label(&input.label)?;
                // Domain error ahead of UNIQUE(project_id, key); the constraint is the backstop.
                let duplicate =
                    sqlx::query("SELECT 1 FROM task_types WHERE project_id = ?1 AND key = ?2")
                        .bind(project.to_string())
                        .bind(&key)
                        .fetch_optional(&mut **tx)
                        .await?;
                if duplicate.is_some() {
                    return Err(DomainError::DuplicateTaskTypeKey { key });
                }
                let id = TaskTypeId::generate(ctx.now);
                sqlx::query(
                    "INSERT INTO task_types (id, revision, created_at, updated_at, project_id, \
                     key, label, archived, builtin) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(format_ts(&ctx.now))
                .bind(project.to_string())
                .bind(&key)
                .bind(&label)
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "createTaskType",
                    vec![PendingEvent::task_type(id, Revision::INITIAL)],
                )
                .await?;
                let task_type = find_task_type(tx, &project, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("task type"))?;
                Ok(CommandResult {
                    value: task_type,
                    events,
                })
            })
        })
        .await
    }

    // Keys are immutable; this renames the label only. Archiving arrives in step 008.
    pub async fn update_task_type(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task_type: TaskTypeId,
        patch: TaskTypePatch,
    ) -> Result<CommandResult<TaskType>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = find_task_type(tx, &project, &task_type)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let label = match &patch.label {
                    Some(value) => validate_type_label(value)?,
                    None => current.label,
                };
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE task_types SET revision = ?1, updated_at = ?2, label = ?3 \
                     WHERE id = ?4",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(&label)
                .bind(task_type.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "updateTaskType",
                    vec![PendingEvent::task_type(task_type, next)],
                )
                .await?;
                let updated = find_task_type(tx, &project, &task_type)
                    .await?
                    .ok_or_else(|| missing_after_write("task type"))?;
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
    use crate::model::{Actor, ActorKind, Clock, ReviewPolicy, TestClock};
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{store_options, test_clock};

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

    async fn project(f: &Fixture, name: &str) -> Project {
        f.store
            .create_project(
                ctx(&f.owner, &f.clock, None),
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

    #[tokio::test]
    async fn create_project_seeds_builtin_types_and_events() {
        let f = fixture().await;
        let created = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: " Space Game ".to_string(),
                    description: Some("desc".to_string()),
                    settings: Some(ProjectSettings {
                        proposal_gate: false,
                        planning_required: true,
                        plan_review: ReviewPolicy::None,
                        work_review: ReviewPolicy::Agent,
                    }),
                },
            )
            .await
            .unwrap();
        assert_eq!(created.value.name, "Space Game");
        assert_eq!(created.value.revision.value(), 1);
        assert!(!created.value.settings.proposal_gate);
        assert_eq!(created.value.epic_counts, crate::model::Counts::ZERO);
        assert_eq!(created.events.len(), 7);

        let types: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM task_types WHERE project_id = ?1 AND builtin = 1",
        )
        .bind(created.value.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(types, i64::try_from(BUILTIN_TASK_TYPES.len()).unwrap());
    }

    #[tokio::test]
    async fn create_project_rejects_agents_revoked_and_unregistered_actors() {
        let f = fixture().await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let mut revoked = person(&f.clock, ActorKind::Human, "revoked");
        revoked.revoked = true;
        register(&f.store, &revoked).await;
        let unregistered = person(&f.clock, ActorKind::Human, "ghost");

        for actor in [&agent, &revoked, &unregistered] {
            let err = f
                .store
                .create_project(
                    ctx(actor, &f.clock, None),
                    ProjectCreate {
                        name: "Rogue".to_string(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::Forbidden(_)), "{}", actor.label);
        }
    }

    #[tokio::test]
    async fn create_project_validates_name_and_description() {
        let f = fixture().await;
        let long_name = "x".repeat(NAME_MAX_CHARS + 1);
        for name in ["   ", long_name.as_str()] {
            let err = f
                .store
                .create_project(
                    ctx(&f.owner, &f.clock, None),
                    ProjectCreate {
                        name: name.to_string(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::Validation { field: "name", .. }));
        }
        let err = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "ok".to_string(),
                    description: Some("d".repeat(DESCRIPTION_MAX_CHARS + 1)),
                    settings: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "description",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn update_project_enforces_revision_and_applies_full_patch() {
        let f = fixture().await;
        let project = project(&f, "P").await;

        let err = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(9)),
                project.id,
                ProjectPatch::default(),
            )
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
            .update_project(
                ctx(&f.owner, &f.clock, None),
                project.id,
                ProjectPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PreconditionRequired));
        let err = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(1)),
                ProjectId::generate(f.clock.now()),
                ProjectPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        f.clock.advance(chrono::TimeDelta::seconds(1));
        let updated = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                ProjectPatch {
                    name: Some("P2".to_string()),
                    description: Some("described".to_string()),
                    settings: Some(ProjectSettings::default()),
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(updated.revision.value(), 2);
        assert_eq!(updated.name, "P2");
        assert_eq!(updated.updated_at, f.clock.now());

        // An empty patch is still a command: apply and bump exactly once.
        let bumped = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(2)),
                project.id,
                ProjectPatch::default(),
            )
            .await
            .unwrap()
            .value;
        assert_eq!(bumped.revision.value(), 3);
        assert_eq!(bumped.name, "P2");
    }

    #[tokio::test]
    async fn mutations_beneath_an_archived_project_are_rejected() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        sqlx::query(
            "UPDATE projects SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'wrapped up', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(project.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                ProjectPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = f
            .store
            .create_goal(
                ctx(&f.owner, &f.clock, None),
                project.id,
                GoalCreate {
                    title: "Late".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = f
            .store
            .create_task_type(
                ctx(&f.owner, &f.clock, None),
                project.id,
                TaskTypeCreate {
                    key: "late".to_string(),
                    label: "Late".to_string(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        // The goal beneath the archived project is likewise frozen once itself archived.
        sqlx::query(
            "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'wrapped up', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(goal.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();
        let err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                goal.id,
                TextPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn create_goal_requires_an_existing_project() {
        let f = fixture().await;
        let err = f
            .store
            .create_goal(
                ctx(&f.owner, &f.clock, None),
                ProjectId::generate(f.clock.now()),
                GoalCreate {
                    title: "Orphan".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));

        let project = project(&f, "P").await;
        let created = goal(&f, project.id, "G").await;
        assert_eq!(created.project_id, project.id);
        assert!(!created.completed);
    }

    #[tokio::test]
    async fn update_goal_scopes_by_project_and_checks_revision() {
        let f = fixture().await;
        let project_a = project(&f, "A").await;
        let project_b = project(&f, "B").await;
        let owned = goal(&f, project_a.id, "In A").await;

        let err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project_b.id,
                owned.id,
                TextPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(7)),
                project_a.id,
                owned.id,
                TextPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::RevisionConflict { .. }));
        let err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project_a.id,
                owned.id,
                TextPatch {
                    title: Some("  ".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "title", .. }
        ));

        let updated = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project_a.id,
                owned.id,
                TextPatch {
                    title: Some("Renamed".to_string()),
                    description: Some("body".to_string()),
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(updated.revision.value(), 2);
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.description, "body");
    }

    #[tokio::test]
    async fn task_type_commands_enforce_key_rules_and_revisions() {
        let f = fixture().await;
        let project = project(&f, "P").await;

        let err = f
            .store
            .create_task_type(
                ctx(&f.owner, &f.clock, None),
                project.id,
                TaskTypeCreate {
                    key: "Bad".to_string(),
                    label: "Bad".to_string(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "key", .. }));

        let custom = f
            .store
            .create_task_type(
                ctx(&f.owner, &f.clock, None),
                project.id,
                TaskTypeCreate {
                    key: "ops".to_string(),
                    label: "Ops".to_string(),
                },
            )
            .await
            .unwrap()
            .value;
        assert!(!custom.builtin);

        for key in ["code", "ops"] {
            let err = f
                .store
                .create_task_type(
                    ctx(&f.owner, &f.clock, None),
                    project.id,
                    TaskTypeCreate {
                        key: key.to_string(),
                        label: "Again".to_string(),
                    },
                )
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::DuplicateTaskTypeKey { key: dup } if dup == key));
        }

        let err = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                TaskTypeId::generate(f.clock.now()),
                TaskTypePatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(5)),
                project.id,
                custom.id,
                TaskTypePatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::RevisionConflict { .. }));

        let renamed = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                custom.id,
                TaskTypePatch {
                    label: Some("Operations".to_string()),
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(renamed.revision.value(), 2);
        assert_eq!(renamed.label, "Operations");
        assert_eq!(renamed.key, "ops");

        // Archived registry entries are frozen until step 008 owns their lifecycle.
        sqlx::query("UPDATE task_types SET archived = 1 WHERE id = ?1")
            .bind(custom.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let err = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(2)),
                project.id,
                custom.id,
                TaskTypePatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }
}
