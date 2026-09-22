use super::{
    CommandContext, CommandResult, PendingEvent, append_events, live_actor, require_owner,
    require_revision,
};
use crate::error::DomainError;
use crate::model::{
    BUILTIN_TASK_TYPES, DESCRIPTION_MAX_CHARS, Epic, EpicCreate, EpicId, EpicStatus, Goal,
    GoalCreate, GoalId, NAME_MAX_CHARS, Project, ProjectCreate, ProjectId, ProjectPatch,
    ProjectSettings, Revision, Task, TaskCreate, TaskId, TaskPatch, TaskType, TaskTypeCreate,
    TaskTypeId, TaskTypePatch, TextPatch, validate_long_text, validate_required_text,
    validate_type_key, validate_type_label,
};
use crate::queries::hierarchy::{
    epic_row, find_epic, find_goal, find_project, find_task, find_task_type, goal_row,
    project_archived, project_row, task_row,
};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store};
use crate::workflow::policy;

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
                let current = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                if patch.name.is_none() && patch.description.is_none() && patch.settings.is_none() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
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
                let scope_archived = project_archived(tx, &project).await?;
                require_owner(&actor)?;
                if scope_archived {
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
                let current = goal_row(tx, &project, &goal)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                if current.archived || project_archived(tx, &project).await? {
                    return Err(DomainError::ArchivedScope);
                }
                if patch.title.is_none() && patch.description.is_none() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
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
                let scope_archived = project_archived(tx, &project).await?;
                require_owner(&actor)?;
                if scope_archived {
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

    // No require_owner: agents create epics (proposed via proposal_gate).
    pub async fn create_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        goal: GoalId,
        input: EpicCreate,
    ) -> Result<CommandResult<Epic>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let proj = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if proj.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let goal_current = goal_row(tx, &project, &goal)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if goal_current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let title = validate_required_text("title", &input.title, NAME_MAX_CHARS)?;
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                // Agents get Proposed when proposal_gate is true; humans always get Open.
                let status = policy::initial_epic_status(actor.kind, proj.settings.proposal_gate);

                let id = EpicId::generate(ctx.now);
                let now_text = format_ts(&ctx.now);
                sqlx::query(
                    "INSERT INTO epics (id, revision, created_at, updated_at, project_id, \
                     goal_id, title, description, status, archived) \
                     VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(&now_text)
                .bind(project.to_string())
                .bind(goal.to_string())
                .bind(&title)
                .bind(&description)
                .bind(status.as_str())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "createEpic",
                    vec![PendingEvent::epic(id, Revision::INITIAL)],
                )
                .await?;
                let epic = find_epic(tx, &project, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("epic"))?;
                Ok(CommandResult {
                    value: epic,
                    events,
                })
            })
        })
        .await
    }

    pub async fn update_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        patch: TextPatch,
    ) -> Result<CommandResult<Epic>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = epic_row(tx, &project, &epic)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_owner(&actor)?;
                require_revision(ctx.expected_revision, current.revision)?;
                let goal_current = goal_row(tx, &project, &current.goal_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if current.archived
                    || goal_current.archived
                    || project_archived(tx, &project).await?
                {
                    return Err(DomainError::ArchivedScope);
                }
                if current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if patch.title.is_none() && patch.description.is_none() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
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
                    "UPDATE epics SET revision = ?1, updated_at = ?2, title = ?3, \
                     description = ?4 WHERE id = ?5",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(&title)
                .bind(&description)
                .bind(epic.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "updateEpic",
                    vec![PendingEvent::epic(epic, next)],
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

    pub async fn accept_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
    ) -> Result<CommandResult<Epic>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                require_owner(&actor)?;
                let current = epic_row(tx, &project, &epic)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_revision(ctx.expected_revision, current.revision)?;
                let goal_current = goal_row(tx, &project, &current.goal_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if current.archived
                    || goal_current.archived
                    || project_archived(tx, &project).await?
                {
                    return Err(DomainError::ArchivedScope);
                }
                if current.status != EpicStatus::Proposed {
                    return Err(DomainError::InvalidState(format!(
                        "epic status is {} but must be proposed",
                        current.status.as_str()
                    )));
                }
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE epics SET revision = ?1, updated_at = ?2, status = ?3 \
                     WHERE id = ?4",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(EpicStatus::Open.as_str())
                .bind(epic.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "acceptEpic",
                    vec![PendingEvent::epic(epic, next)],
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

    // No require_owner: agents create tasks (proposed via proposal_gate).
    pub async fn create_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        input: TaskCreate,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let proj = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if proj.archived {
                    return Err(DomainError::ArchivedScope);
                }
                let epic_current = epic_row(tx, &project, &epic)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if epic_current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                if epic_current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                let title = validate_required_text("title", &input.title, NAME_MAX_CHARS)?;
                let description = input.description.clone().unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;

                // Validate type_key exists and is not archived.
                let type_check: Option<(i64,)> = sqlx::query_as(
                    "SELECT archived FROM task_types WHERE project_id = ?1 AND key = ?2",
                )
                .bind(project.to_string())
                .bind(&input.type_key)
                .fetch_optional(&mut **tx)
                .await?;
                match type_check {
                    None => {
                        return Err(DomainError::Validation {
                            field: "type_key",
                            message: format!("task type '{}' does not exist", input.type_key),
                        });
                    }
                    Some((1,)) => {
                        return Err(DomainError::Validation {
                            field: "type_key",
                            message: format!("task type '{}' is archived", input.type_key),
                        });
                    }
                    _ => {}
                }

                let resolved = policy::resolve_task_policy(&proj.settings, actor.kind, &input)?;
                let phase = policy::initial_phase(resolved.planning_required);
                let status = policy::initial_status(actor.kind, proj.settings.proposal_gate);

                let id = TaskId::generate(ctx.now);
                let now_text = format_ts(&ctx.now);
                sqlx::query(
                    "INSERT INTO tasks (id, revision, created_at, updated_at, project_id, \
                     epic_id, title, description, type_key, status, phase, \
                     planning_required, plan_review, work_review, archived, attempt_count) \
                     VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(&now_text)
                .bind(project.to_string())
                .bind(epic.to_string())
                .bind(&title)
                .bind(&description)
                .bind(&input.type_key)
                .bind(status.as_str())
                .bind(phase.as_str())
                .bind(i64::from(resolved.planning_required))
                .bind(resolved.plan_review.as_str())
                .bind(resolved.work_review.as_str())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "createTask",
                    vec![PendingEvent::task(id, Revision::INITIAL)],
                )
                .await?;
                let task = find_task(tx, &project, &id)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;
                Ok(CommandResult {
                    value: task,
                    events,
                })
            })
        })
        .await
    }

    pub async fn update_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
        patch: TaskPatch,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                let current = task_row(tx, &project, &task)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                // Capability before revision (plan/04 failure precedence).
                let has_policy_change = patch.planning_required.is_some()
                    || patch.plan_review.is_some()
                    || patch.work_review.is_some();
                if has_policy_change {
                    require_owner(&actor)?;
                }
                require_revision(ctx.expected_revision, current.revision)?;
                let proj = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let epic_current = epic_row(tx, &project, &current.epic_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if current.archived || epic_current.archived || proj.archived {
                    return Err(DomainError::ArchivedScope);
                }
                if current.status.is_terminal() || epic_current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if patch.is_empty() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
                }

                let title = match &patch.title {
                    Some(value) => validate_required_text("title", value, NAME_MAX_CHARS)?,
                    None => current.title.clone(),
                };
                let description = match &patch.description {
                    Some(value) => {
                        validate_long_text("description", value, DESCRIPTION_MAX_CHARS)?;
                        value.clone()
                    }
                    None => current.description.clone(),
                };
                let type_key = match &patch.type_key {
                    Some(new_key) => {
                        let type_check: Option<(i64,)> = sqlx::query_as(
                            "SELECT archived FROM task_types \
                             WHERE project_id = ?1 AND key = ?2",
                        )
                        .bind(project.to_string())
                        .bind(new_key)
                        .fetch_optional(&mut **tx)
                        .await?;
                        match type_check {
                            None => {
                                return Err(DomainError::Validation {
                                    field: "type_key",
                                    message: format!("task type '{new_key}' does not exist"),
                                });
                            }
                            Some((1,)) => {
                                return Err(DomainError::Validation {
                                    field: "type_key",
                                    message: format!("task type '{new_key}' is archived"),
                                });
                            }
                            _ => {}
                        }
                        new_key.clone()
                    }
                    None => current.type_key.clone(),
                };
                let planning_required =
                    patch.planning_required.unwrap_or(current.planning_required);
                let plan_review = patch.plan_review.unwrap_or(current.plan_review);
                let work_review = patch.work_review.unwrap_or(current.work_review);
                // Only enforce agent floor when the patch touches policy fields;
                // an agent renaming a human-lowered task must not be rejected.
                if has_policy_change {
                    policy::validate_policy_update(
                        &proj.settings,
                        actor.kind,
                        planning_required,
                        plan_review,
                        work_review,
                    )?;
                }

                // Actual value change (not mere presence) triggers phase reset.
                let policy_actually_changed = description != current.description
                    || planning_required != current.planning_required
                    || plan_review != current.plan_review
                    || work_review != current.work_review;
                let phase = if policy_actually_changed {
                    policy::initial_phase(planning_required)
                } else {
                    current.phase
                };

                let next = current.revision.next();
                sqlx::query(
                    "UPDATE tasks SET revision = ?1, updated_at = ?2, title = ?3, \
                     description = ?4, type_key = ?5, planning_required = ?6, \
                     plan_review = ?7, work_review = ?8, phase = ?9, \
                     selected_plan_revision_id = CASE WHEN ?10 THEN NULL \
                        ELSE selected_plan_revision_id END, \
                     accepted_plan_submission_id = CASE WHEN ?10 THEN NULL \
                        ELSE accepted_plan_submission_id END \
                     WHERE id = ?11",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(&title)
                .bind(&description)
                .bind(&type_key)
                .bind(i64::from(planning_required))
                .bind(plan_review.as_str())
                .bind(work_review.as_str())
                .bind(phase.as_str())
                .bind(policy_actually_changed)
                .bind(task.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "updateTask",
                    vec![PendingEvent::task(task, next)],
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

    pub async fn accept_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &ctx.actor.id).await?;
                require_owner(&actor)?;
                let current = task_row(tx, &project, &task)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_revision(ctx.expected_revision, current.revision)?;
                let epic_current = epic_row(tx, &project, &current.epic_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if current.archived
                    || epic_current.archived
                    || project_archived(tx, &project).await?
                {
                    return Err(DomainError::ArchivedScope);
                }
                if epic_current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if current.status != crate::model::TaskStatus::Proposed {
                    return Err(DomainError::InvalidState(format!(
                        "task status is {} but must be proposed",
                        current.status.as_str()
                    )));
                }
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE tasks SET revision = ?1, updated_at = ?2, status = ?3 \
                     WHERE id = ?4",
                )
                .bind(next.value())
                .bind(format_ts(&ctx.now))
                .bind(crate::model::TaskStatus::Open.as_str())
                .bind(task.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &ctx,
                    "acceptTask",
                    vec![PendingEvent::task(task, next)],
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
                if current.archived || project_archived(tx, &project).await? {
                    return Err(DomainError::ArchivedScope);
                }
                if patch.label.is_none() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
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

        // An all-None patch is rejected: no revision bump, no event.
        let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        let err = f
            .store
            .update_project(
                ctx(&f.owner, &f.clock, Some(2)),
                project.id,
                ProjectPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "patch", .. }
        ));
        let (revision, name): (i64, String) =
            sqlx::query_as("SELECT revision, name FROM projects WHERE id = ?1")
                .bind(project.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(revision, 2);
        assert_eq!(name, "P2");
        let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(events_after, events_before);
    }

    #[tokio::test]
    async fn mutations_beneath_an_archived_project_are_rejected() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let code_type: TaskTypeId =
            sqlx::query_scalar("SELECT id FROM task_types WHERE project_id = ?1 AND key = 'code'")
                .bind(project.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .map(|id: String| id.parse().unwrap())
                .unwrap();
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

        // Live children beneath the archived project are frozen too.
        let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        let err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                goal.id,
                TextPatch {
                    title: Some("Renamed".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let err = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                code_type,
                TaskTypePatch {
                    label: Some("Renamed".to_string()),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
        let (goal_revision, goal_title): (i64, String) =
            sqlx::query_as("SELECT revision, title FROM goals WHERE id = ?1")
                .bind(goal.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(goal_revision, 1);
        assert_eq!(goal_title, "G");
        let type_revisions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM task_types WHERE revision != 1")
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(type_revisions, 0);
        let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(events_after, events_before);

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
    async fn empty_patches_are_rejected_without_writes() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let code_type: TaskTypeId =
            sqlx::query_scalar("SELECT id FROM task_types WHERE project_id = ?1 AND key = 'code'")
                .bind(project.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .map(|id: String| id.parse().unwrap())
                .unwrap();

        let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        let goal_err = f
            .store
            .update_goal(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                goal.id,
                TextPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            goal_err,
            DomainError::Validation { field: "patch", .. }
        ));
        let type_err = f
            .store
            .update_task_type(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                code_type,
                TaskTypePatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            type_err,
            DomainError::Validation { field: "patch", .. }
        ));

        let goal_revision: i64 = sqlx::query_scalar("SELECT revision FROM goals WHERE id = ?1")
            .bind(goal.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(goal_revision, 1);
        let type_revision: i64 =
            sqlx::query_scalar("SELECT revision FROM task_types WHERE id = ?1")
                .bind(code_type.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(type_revision, 1);
        let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(events_after, events_before);
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

    #[tokio::test]
    async fn create_epic_seeds_fields_and_events() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let created = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project.id,
                goal.id,
                EpicCreate {
                    title: " Epic One ".to_string(),
                    description: Some("desc".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(created.value.title, "Epic One");
        assert_eq!(created.value.description, "desc");
        assert_eq!(created.value.revision.value(), 1);
        assert_eq!(created.value.status, EpicStatus::Open);
        assert_eq!(created.events.len(), 1);
    }

    #[tokio::test]
    async fn create_epic_validates_and_rejects_bad_input() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;

        let err = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project.id,
                goal.id,
                EpicCreate {
                    title: "   ".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "title", .. }
        ));

        let fake_goal = crate::model::GoalId::generate(f.clock.now());
        let err = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project.id,
                fake_goal,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn update_epic_validates_revision_and_terminal() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;

        let err = f
            .store
            .update_epic(
                ctx(&f.owner, &f.clock, Some(9)),
                project.id,
                e.id,
                TextPatch {
                    title: Some("X".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::RevisionConflict { .. }));

        f.clock.advance(chrono::TimeDelta::seconds(1));
        let updated = f
            .store
            .update_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                e.id,
                TextPatch {
                    title: Some("Renamed".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(updated.revision.value(), 2);
        assert_eq!(updated.title, "Renamed");
    }

    #[tokio::test]
    async fn accept_epic_checks_status_and_transitions() {
        let f = fixture().await;
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
        let goal = goal(&f, project.id, "G").await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let e = f
            .store
            .create_epic(
                ctx(&agent, &f.clock, None),
                project.id,
                goal.id,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(e.status, EpicStatus::Proposed);

        f.clock.advance(chrono::TimeDelta::seconds(1));
        let accepted = f
            .store
            .accept_epic(ctx(&f.owner, &f.clock, Some(1)), project.id, e.id)
            .await
            .unwrap()
            .value;
        assert_eq!(accepted.status, EpicStatus::Open);
        assert_eq!(accepted.revision.value(), 2);

        let err = f
            .store
            .accept_epic(ctx(&f.owner, &f.clock, Some(2)), project.id, e.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    #[tokio::test]
    async fn create_task_snapshots_policy_and_validates_type() {
        let f = fixture().await;
        let project = f
            .store
            .create_project(
                ctx(&f.owner, &f.clock, None),
                ProjectCreate {
                    name: "P".to_string(),
                    settings: Some(ProjectSettings {
                        proposal_gate: false,
                        planning_required: true,
                        plan_review: ReviewPolicy::Agent,
                        work_review: ReviewPolicy::Human,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;

        let t = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        assert!(t.planning_required);
        assert_eq!(t.plan_review, ReviewPolicy::Agent);
        assert_eq!(t.work_review, ReviewPolicy::Human);
        assert_eq!(t.phase, crate::model::TaskPhase::Planning);
        assert_eq!(t.status, crate::model::TaskStatus::Open);

        let err = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "Bad".to_string(),
                    type_key: "nonexistent".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "type_key",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn update_task_enforces_revision_and_policy() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t = task(&f, project.id, e.id, "T").await;

        let err = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(9)),
                project.id,
                t.id,
                TaskPatch {
                    title: Some("X".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::RevisionConflict { .. }));

        f.clock.advance(chrono::TimeDelta::seconds(1));
        let updated = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                t.id,
                TaskPatch {
                    title: Some("Renamed".to_string()),
                    description: Some("new desc".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(updated.revision.value(), 2);
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.description, "new desc");
    }

    #[tokio::test]
    async fn accept_task_checks_status_and_transitions() {
        let f = fixture().await;
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
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let t = f
            .store
            .create_task(
                ctx(&agent, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(t.status, crate::model::TaskStatus::Proposed);

        f.clock.advance(chrono::TimeDelta::seconds(1));
        let accepted = f
            .store
            .accept_task(ctx(&f.owner, &f.clock, Some(1)), project.id, t.id)
            .await
            .unwrap()
            .value;
        assert_eq!(accepted.status, crate::model::TaskStatus::Open);
        assert_eq!(accepted.revision.value(), 2);
    }

    #[tokio::test]
    async fn epic_mutations_check_archived_and_terminal_scope() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;

        // Archive the goal.
        sqlx::query(
            "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(goal.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .update_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                e.id,
                TextPatch {
                    title: Some("X".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));

        // Restore goal, cancel epic.
        sqlx::query(
            "UPDATE goals SET archived = 0, archive_actor_id = NULL, \
             archive_reason = NULL, archive_created_at = NULL WHERE id = ?1",
        )
        .bind(goal.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "UPDATE epics SET status = 'cancelled', cancellation_actor_id = ?1, \
             cancellation_reason = 'scrapped', cancellation_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(e.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .update_epic(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                e.id,
                TextPatch {
                    title: Some("X".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn task_mutations_check_archived_and_terminal_scope() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t = task(&f, project.id, e.id, "T").await;

        // Archive the epic.
        sqlx::query(
            "UPDATE epics SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(e.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                t.id,
                TaskPatch {
                    title: Some("X".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));

        // Restore epic, cancel it.
        sqlx::query(
            "UPDATE epics SET archived = 0, archive_actor_id = NULL, \
             archive_reason = NULL, archive_created_at = NULL, \
             status = 'cancelled', cancellation_actor_id = ?1, \
             cancellation_reason = 'done', cancellation_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(e.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                t.id,
                TaskPatch {
                    title: Some("X".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn create_epic_rejects_archived_goal_and_project() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;

        sqlx::query(
            "UPDATE goals SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(goal.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .create_epic(
                ctx(&f.owner, &f.clock, None),
                project.id,
                goal.id,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));
    }

    #[tokio::test]
    async fn create_task_rejects_archived_and_terminal_epic() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;

        sqlx::query(
            "UPDATE epics SET archived = 1, archive_actor_id = ?1, \
             archive_reason = 'done', archive_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(e.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ArchivedScope));

        sqlx::query(
            "UPDATE epics SET archived = 0, archive_actor_id = NULL, \
             archive_reason = NULL, archive_created_at = NULL, \
             status = 'done' WHERE id = ?1",
        )
        .bind(e.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let err = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn create_task_rejects_archived_type_key() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;

        sqlx::query("UPDATE task_types SET archived = 1 WHERE project_id = ?1 AND key = 'design'")
            .bind(project.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();

        let err = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                project.id,
                e.id,
                TaskCreate {
                    title: "T".to_string(),
                    type_key: "design".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "type_key",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn update_task_empty_patch_and_policy_change_require_owner() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t = task(&f, project.id, e.id, "T").await;

        let err = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                t.id,
                TaskPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "patch", .. }
        ));

        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let err = f
            .store
            .update_task(
                ctx(&agent, &f.clock, Some(1)),
                project.id,
                t.id,
                TaskPatch {
                    plan_review: Some(ReviewPolicy::None),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }
}
