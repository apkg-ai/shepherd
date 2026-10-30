use super::*;

impl Store {
    // No capability guard: agents create tasks (proposed via proposal_gate).
    pub async fn create_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        input: TaskCreate,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 201, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
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
                let goal_current = goal_row(tx, &project, &epic_current.goal_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if goal_current.archived {
                    return Err(DomainError::ArchivedScope);
                }
                if epic_current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                let title = validate_required_text("title", &input.title, NAME_MAX_CHARS)?;

                // Validate format before DB lookup; safe to echo in errors after this.
                let validated_key = validate_type_key(&input.type_key)?;
                let type_check: Option<(i64,)> = sqlx::query_as(
                    "SELECT archived FROM task_types WHERE project_id = ?1 AND key = ?2",
                )
                .bind(project.to_string())
                .bind(&validated_key)
                .fetch_optional(&mut **tx)
                .await?;
                match type_check {
                    None => {
                        return Err(DomainError::Validation {
                            field: "type_key",
                            message: format!("task type '{validated_key}' does not exist"),
                        });
                    }
                    Some((1,)) => {
                        return Err(DomainError::Validation {
                            field: "type_key",
                            message: format!("task type '{validated_key}' is archived"),
                        });
                    }
                    _ => {}
                }

                let resolved = policy::resolve_task_policy(&proj.settings, actor.kind, &input)?;
                // Extract description after policy resolution to avoid cloning input.
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                let phase = policy::initial_phase(resolved.planning_required);
                let status = policy::initial_status(actor.kind, proj.settings.proposal_gate);

                let id = TaskId::generate(closure_ctx.now);
                let now_text = format_ts(&closure_ctx.now);
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
                .bind(&validated_key)
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
                    &closure_ctx,
                    "createTask",
                    "",
                    vec![PendingEvent::task(id, Revision::INITIAL, epic)],
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
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let current = task_row(tx, &project, &task)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                // Capability before revision (plan/04 failure precedence).
                let has_policy_change = patch.planning_required.is_some()
                    || patch.plan_review.is_some()
                    || patch.work_review.is_some();
                if has_policy_change {
                    require_capability(&actor, Capability::LowerRequirements)?;
                }
                require_revision(closure_ctx.expected_revision, current.revision)?;
                let proj = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let epic_current = epic_row(tx, &project, &current.epic_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                let goal_current = goal_row(tx, &project, &epic_current.goal_id)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                if current.archived
                    || epic_current.archived
                    || goal_current.archived
                    || proj.archived
                {
                    return Err(DomainError::ArchivedScope);
                }
                if current.status.is_terminal() || epic_current.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                // No edit while a claim is held or a review is pending (plan/03).
                let now_text = format_ts(&closure_ctx.now);
                if task_has_active_work(tx, task, &now_text).await? {
                    return Err(DomainError::ActiveWork(
                        "task has an active claim or pending submission".into(),
                    ));
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
                        let validated = validate_type_key(new_key)?;
                        let type_check: Option<(i64,)> = sqlx::query_as(
                            "SELECT archived FROM task_types \
                             WHERE project_id = ?1 AND key = ?2",
                        )
                        .bind(project.to_string())
                        .bind(&validated)
                        .fetch_optional(&mut **tx)
                        .await?;
                        match type_check {
                            None => {
                                return Err(DomainError::Validation {
                                    field: "type_key",
                                    message: format!("task type '{validated}' does not exist"),
                                });
                            }
                            Some((1,)) => {
                                return Err(DomainError::Validation {
                                    field: "type_key",
                                    message: format!("task type '{validated}' is archived"),
                                });
                            }
                            _ => {}
                        }
                        validated
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
                .bind(format_ts(&closure_ctx.now))
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
                    &closure_ctx,
                    "updateTask",
                    "",
                    vec![PendingEvent::task(task, next, current.epic_id)],
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
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                // Membership precedes capability (plan/04): a missing or foreign task is 404.
                let scope = task_scope(tx, &project, &task).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if scope.task.status != crate::model::TaskStatus::Proposed {
                    return Err(DomainError::InvalidState(format!(
                        "task status is {} but must be proposed",
                        scope.task.status.as_str()
                    )));
                }
                let next = scope.task.revision.next();
                sqlx::query(
                    "UPDATE tasks SET revision = ?1, updated_at = ?2, status = ?3 \
                     WHERE id = ?4",
                )
                .bind(next.value())
                .bind(format_ts(&closure_ctx.now))
                .bind(TaskStatus::Open.as_str())
                .bind(task.to_string())
                .execute(&mut **tx)
                .await?;
                // The plan/04 cascade is provably a no-op here: counts already
                // include proposed tasks and task blocks never gate epic
                // completion, so accepting a task cannot flip any epic state.
                let pending = vec![PendingEvent::task(task, next, scope.task.epic_id)];
                let events =
                    append_events(tx, &project, &closure_ctx, "acceptTask", "", pending).await?;
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

    // Owner or agent (plan/12): blocking needs no owner capability.
    pub async fn block_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
        reason: String,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if scope.task.block.is_some() {
                    return Err(DomainError::InvalidState("task is already blocked".into()));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::task::block(tx, &closure_ctx, &scope.task, &reason, &mut pending).await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "blockTask", &reason, pending)
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

    pub async fn unblock_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_capability(&actor, Capability::RemoveDependencyOrUnblock)?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                let Some(block) = &scope.task.block else {
                    return Err(DomainError::InvalidState("task is not blocked".into()));
                };
                // The unblock event preserves the cleared block reason (plan/08).
                let cleared = block.reason.clone();
                // The plan/04 cascade is provably a no-op here: a task block
                // never gates epic completion, so clearing it cannot flip any
                // epic state.
                let mut pending = Vec::new();
                workflow::task::unblock(tx, &closure_ctx, &scope.task, &mut pending).await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "unblockTask", &cleared, pending)
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

    pub async fn cancel_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
        reason: String,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::task::cancel(tx, &closure_ctx, &scope.task, &reason, &mut pending)
                    .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "cancelTask", &reason, pending)
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

    pub async fn waive_task(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
        reason: String,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if scope.task.status != TaskStatus::Cancelled {
                    return Err(DomainError::InvalidState(format!(
                        "task status is {} but must be cancelled",
                        scope.task.status.as_str()
                    )));
                }
                if scope.task.waiver.is_some() {
                    return Err(DomainError::InvalidState("task is already waived".into()));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::task::waive(tx, &closure_ctx, &scope.task, &reason, &mut pending).await?;
                // The waiver changes the epic's completion denominator: bump it
                // once here; the cascade reuses this revision if it completes.
                let epic_next = scope.epic.revision.next();
                sqlx::query("UPDATE epics SET revision = ?1, updated_at = ?2 WHERE id = ?3")
                    .bind(epic_next.value())
                    .bind(format_ts(&closure_ctx.now))
                    .bind(scope.task.epic_id.to_string())
                    .execute(&mut **tx)
                    .await?;
                pending.push(PendingEvent::epic(
                    scope.task.epic_id,
                    epic_next,
                    scope.epic.goal_id,
                ));
                workflow::recompute(
                    tx,
                    AffectedScope {
                        project,
                        epics: vec![scope.task.epic_id],
                    },
                    &mut pending,
                    closure_ctx.now,
                )
                .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "waiveTask", &reason, pending)
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

    // Test-only driver for the done transition and cascade; production path is reportClaim.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn complete_task_for_test(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task: TaskId,
    ) -> Result<CommandResult<Task>, DomainError> {
        self.domain_transaction(move |tx| {
            Box::pin(async move {
                live_actor(tx, &ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task).await?;
                require_revision(ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                // A blocked task can never report done in production; epic-level
                // blocks stay permitted so tests can build the reachable
                // children-done-before-block state.
                if scope.task.block.is_some() {
                    return Err(DomainError::InvalidState("task is blocked".into()));
                }
                let mut pending = Vec::new();
                workflow::task::complete(tx, &ctx, &scope.task, &mut pending).await?;
                workflow::recompute(
                    tx,
                    AffectedScope {
                        project,
                        epics: vec![scope.task.epic_id],
                    },
                    &mut pending,
                    ctx.now,
                )
                .await?;
                let events =
                    append_events(tx, &project, &ctx, "system.test.completeTask", "", pending)
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
    use super::super::test_support::*;
    use super::*;

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

    #[tokio::test]
    async fn update_task_rejects_active_claims_and_pending_reviews() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let goal = goal(&f, project.id, "G").await;
        let e = epic(&f, project.id, goal.id, "E").await;
        let t = task(&f, project.id, e.id, "T").await;
        let patch = TaskPatch {
            title: Some("Renamed".to_string()),
            ..Default::default()
        };
        // Direct-SQL fixture: claim and submission commands land in steps 009/011.
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
         status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'execute', ?4, ?5, \
         'active', 1, 'hash')",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(t.id.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(format_ts(&(f.clock.now() + chrono::Duration::minutes(30))))
        .execute(f.store.pool())
        .await
        .unwrap();
        let err = f
            .store
            .update_task(
                ctx(&f.owner, &f.clock, Some(1)),
                project.id,
                t.id,
                patch.clone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::ActiveWork(_)));

        // Expiring exactly now is inactive (plan/04): the edit goes through.
        sqlx::query("UPDATE claims SET expires_at = ?1 WHERE task_id = ?2")
            .bind(format_ts(&f.clock.now()))
            .bind(t.id.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let updated = f
            .store
            .update_task(ctx(&f.owner, &f.clock, Some(1)), project.id, t.id, patch)
            .await
            .unwrap();
        assert_eq!(updated.value.title, "Renamed");
    }

    #[tokio::test]
    async fn block_task_blocks_revokes_claim_and_keeps_submission() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        let claim = seed_claim_row(&f, s.task.id).await;
        seed_submission_row(&f, s.task.id).await;

        let blocked = f
            .store
            .block_task(
                ctx(&agent, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "  scope changed  ".to_string(),
            )
            .await
            .unwrap();
        let block = blocked.value.block.expect("block record");
        assert_eq!(block.reason, "scope changed");
        assert_eq!(block.actor_id, agent.id);
        assert_eq!(blocked.value.revision.value(), 2);
        assert_eq!(
            claim_state(&f, &claim).await,
            ("revoked".into(), "scope changed".into())
        );
        let pending: String =
            sqlx::query_scalar("SELECT status FROM submissions WHERE task_id = ?1")
                .bind(s.task.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(pending, "pending");
        let rows = event_rows(&f, &blocked.events).await;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(_, _, _, action, reason)| action == "blockTask" && reason == "scope changed"));
        assert!(
            rows.iter()
                .any(|(kind, _, _, _, _)| kind == "claim.changed")
        );
        assert!(
            rows.iter()
                .any(|(kind, id, revision, _, _)| kind == "task.changed"
                    && *id == s.task.id.to_string()
                    && *revision == 2)
        );

        let err = f
            .store
            .block_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "again".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    #[tokio::test]
    async fn block_task_validates_reason_and_terminal_scope() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        for reason in ["   ", &"x".repeat(REASON_MAX_CHARS + 1)] {
            let err = f
                .store
                .block_task(
                    ctx(&f.owner, &f.clock, Some(1)),
                    s.project,
                    s.task.id,
                    reason.to_string(),
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
        }
        f.store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "obsolete".into(),
            )
            .await
            .unwrap();
        let err = f
            .store
            .block_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "late".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn unblock_task_is_owner_only_and_preserves_the_cleared_reason() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;

        let err = f
            .store
            .unblock_task(ctx(&f.owner, &f.clock, Some(1)), s.project, s.task.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));

        f.store
            .block_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "waiting on design".into(),
            )
            .await
            .unwrap();
        let err = f
            .store
            .unblock_task(ctx(&agent, &f.clock, Some(2)), s.project, s.task.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        let unblocked = f
            .store
            .unblock_task(ctx(&f.owner, &f.clock, Some(2)), s.project, s.task.id)
            .await
            .unwrap();
        assert!(unblocked.value.block.is_none());
        assert_eq!(unblocked.value.revision.value(), 3);
        let rows = event_rows(&f, &unblocked.events).await;
        // The unblock event preserves the cleared block reason (plan/08).
        assert!(
            rows.iter()
                .all(|(_, _, _, action, reason)| action == "unblockTask"
                    && reason == "waiting on design")
        );
    }

    #[tokio::test]
    async fn cancel_task_completes_phase_revokes_claims_and_withdraws_submissions() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        let claim = seed_claim_row(&f, s.task.id).await;
        let submission = seed_submission_row(&f, s.task.id).await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;

        let err = f
            .store
            .cancel_task(
                ctx(&agent, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "nope".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        let cancelled = f
            .store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "superseded".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            task_state(&f, s.task.id).await,
            ("cancelled".into(), "complete".into(), 2)
        );
        let record = cancelled.value.cancellation.expect("cancellation record");
        assert_eq!(record.reason, "superseded");
        assert_eq!(
            claim_state(&f, &claim).await,
            ("revoked".into(), "superseded".into())
        );
        let (status, withdraw_reason): (String, String) =
            sqlx::query_as("SELECT status, withdraw_reason FROM submissions WHERE id = ?1")
                .bind(&submission)
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(
            (status.as_str(), withdraw_reason.as_str()),
            ("withdrawn", "superseded")
        );
        let rows = event_rows(&f, &cancelled.events).await;
        assert!(
            rows.iter()
                .any(|(kind, _, _, _, _)| kind == "submission.changed")
        );

        let err = f
            .store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "twice".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::TerminalScope));
    }

    #[tokio::test]
    async fn waive_task_needs_owner_a_cancelled_task_and_no_prior_waiver() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;

        let err = f
            .store
            .waive_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "early".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));

        f.store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "obsolete".into(),
            )
            .await
            .unwrap();
        let err = f
            .store
            .waive_task(
                ctx(&agent, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "nope".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        let waived = f
            .store
            .waive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "not needed".into(),
            )
            .await
            .unwrap();
        assert_eq!(waived.value.waiver.expect("waiver").reason, "not needed");
        // The waiver changes the epic's completion denominator: one epic bump.
        assert_eq!(epic_state(&f, s.epic.id).await.1, 2);

        let err = f
            .store
            .waive_task(
                ctx(&f.owner, &f.clock, Some(3)),
                s.project,
                s.task.id,
                "twice".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    #[tokio::test]
    async fn waiving_the_last_cancelled_task_completes_the_epic_once() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        let done = task(&f, s.project, s.epic.id, "Done work").await;
        f.store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), s.project, done.id)
            .await
            .unwrap();
        f.store
            .cancel_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "obsolete".into(),
            )
            .await
            .unwrap();
        assert_eq!(epic_state(&f, s.epic.id).await.0, "open");

        let waived = f
            .store
            .waive_task(
                ctx(&f.owner, &f.clock, Some(2)),
                s.project,
                s.task.id,
                "not needed".into(),
            )
            .await
            .unwrap();
        // Auto-completion fires in the same command and reuses the waiver's
        // epic bump: exactly one epic.changed with the final revision.
        assert_eq!(epic_state(&f, s.epic.id).await, ("done".into(), 2));
        let rows = event_rows(&f, &waived.events).await;
        let epic_events: Vec<_> = rows
            .iter()
            .filter(|(kind, _, _, _, _)| kind == "epic.changed")
            .collect();
        assert_eq!(epic_events.len(), 1);
        assert_eq!(epic_events[0].2, 2);
    }

    #[tokio::test]
    async fn test_driver_rejects_a_blocked_task() {
        let f = fixture().await;
        let s = lifecycle_scope(&f).await;
        f.store
            .block_task(
                ctx(&f.owner, &f.clock, Some(1)),
                s.project,
                s.task.id,
                "hold".into(),
            )
            .await
            .unwrap();
        // A blocked task can never report done in production.
        let err = f
            .store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(2)), s.project, s.task.id)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidState(_)));
    }

    #[tokio::test]
    async fn completing_the_last_task_cascades_across_epic_dependencies() {
        let f = fixture().await;
        let p = project(&f, "P").await;
        let g = goal(&f, p.id, "G").await;
        let first = epic(&f, p.id, g.id, "First").await;
        let first_task = task(&f, p.id, first.id, "A").await;
        let second = epic(&f, p.id, g.id, "Second").await;
        let second_task = task(&f, p.id, second.id, "B").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                p.id,
                DependencyCreate::Epic {
                    dependent_id: second.id,
                    prerequisite_id: first.id,
                },
            )
            .await
            .unwrap();
        f.store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), p.id, second_task.id)
            .await
            .unwrap();
        // Gated by the unmet epic prerequisite.
        assert_eq!(epic_state(&f, second.id).await.0, "open");

        let completed = f
            .store
            .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), p.id, first_task.id)
            .await
            .unwrap();
        assert_eq!(task_state(&f, first_task.id).await.0, "done");
        assert_eq!(epic_state(&f, first.id).await.0, "done");
        assert_eq!(epic_state(&f, second.id).await.0, "done");
        // One transaction, one command: task + both epics, coalesced.
        let rows = event_rows(&f, &completed.events).await;
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .filter(|(kind, _, _, _, _)| kind == "epic.changed")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn create_task_validates_title_and_description() {
        let f = fixture().await;
        let p = project(&f, "P").await;
        let g = goal(&f, p.id, "G").await;
        let e = epic(&f, p.id, g.id, "E").await;

        for title in ["   ", &"x".repeat(NAME_MAX_CHARS + 1)] {
            let err = f
                .store
                .create_task(
                    ctx(&f.owner, &f.clock, None),
                    p.id,
                    e.id,
                    TaskCreate {
                        title: title.to_string(),
                        type_key: "code".to_string(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(
                matches!(err, DomainError::Validation { field: "title", .. }),
                "expected title validation error for {:?}",
                title
            );
        }

        let err = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock, None),
                p.id,
                e.id,
                TaskCreate {
                    title: "Valid".to_string(),
                    type_key: "code".to_string(),
                    description: Some("d".repeat(DESCRIPTION_MAX_CHARS + 1)),
                    ..Default::default()
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
}
