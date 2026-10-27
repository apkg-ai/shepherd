use super::*;

impl Store {
    // No capability guard: agents create epics (proposed via proposal_gate).
    pub async fn create_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        goal: GoalId,
        input: EpicCreate,
    ) -> Result<CommandResult<Epic>, DomainError> {
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

                let id = EpicId::generate(closure_ctx.now);
                let now_text = format_ts(&closure_ctx.now);
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
                    &closure_ctx,
                    "createEpic",
                    "",
                    vec![PendingEvent::epic(id, Revision::INITIAL, goal)],
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

    // No capability guard: writers (agents included) edit nonterminal epics (plan/12).
    pub async fn update_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        patch: TextPatch,
    ) -> Result<CommandResult<Epic>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                live_actor(tx, &closure_ctx.actor.id).await?;
                let current = epic_row(tx, &project, &epic)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_revision(closure_ctx.expected_revision, current.revision)?;
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
                .bind(format_ts(&closure_ctx.now))
                .bind(&title)
                .bind(&description)
                .bind(epic.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "updateEpic",
                    "",
                    vec![PendingEvent::epic(epic, next, current.goal_id)],
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
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                // Membership precedes capability (plan/04): a missing or foreign epic is 404.
                let scope = epic_scope(tx, &project, &epic).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status != EpicStatus::Proposed {
                    return Err(DomainError::InvalidState(format!(
                        "epic status is {} but must be proposed",
                        scope.epic.status.as_str()
                    )));
                }
                let next = scope.epic.revision.next();
                sqlx::query(
                    "UPDATE epics SET revision = ?1, updated_at = ?2, status = ?3 \
                     WHERE id = ?4",
                )
                .bind(next.value())
                .bind(format_ts(&closure_ctx.now))
                .bind(EpicStatus::Open.as_str())
                .bind(epic.to_string())
                .execute(&mut **tx)
                .await?;
                // Accept runs the cascade (plan/04): tasks may already be done.
                let mut pending = vec![PendingEvent::epic(epic, next, scope.epic.goal_id)];
                workflow::recompute(
                    tx,
                    AffectedScope {
                        project,
                        epics: vec![epic],
                    },
                    &mut pending,
                    closure_ctx.now,
                )
                .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "acceptEpic", "", pending).await?;
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

    // Owner or agent (plan/12): blocking needs no owner capability.
    pub async fn block_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        reason: String,
    ) -> Result<CommandResult<Epic>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = epic_scope(tx, &project, &epic).await?;
                require_revision(closure_ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if scope.epic.block.is_some() {
                    return Err(DomainError::InvalidState("epic is already blocked".into()));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::epic::block(tx, &closure_ctx, &scope.epic, &reason, &mut pending).await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "blockEpic", &reason, pending)
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

    pub async fn unblock_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
    ) -> Result<CommandResult<Epic>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = epic_scope(tx, &project, &epic).await?;
                require_capability(&actor, Capability::RemoveDependencyOrUnblock)?;
                require_revision(closure_ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                let Some(block) = &scope.epic.block else {
                    return Err(DomainError::InvalidState("epic is not blocked".into()));
                };
                let cleared = block.reason.clone();
                let mut pending = Vec::new();
                workflow::epic::unblock(tx, &closure_ctx, &scope.epic, &mut pending).await?;
                // Unblocking re-evaluates children finished before the block.
                workflow::recompute(
                    tx,
                    AffectedScope {
                        project,
                        epics: vec![epic],
                    },
                    &mut pending,
                    closure_ctx.now,
                )
                .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "unblockEpic", &cleared, pending)
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

    pub async fn cancel_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        reason: String,
    ) -> Result<CommandResult<Epic>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = epic_scope(tx, &project, &epic).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::epic::cancel(tx, &closure_ctx, &scope.epic, &reason, &mut pending)
                    .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "cancelEpic", &reason, pending)
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

    // Explicit completion covers the empty and all-waived shapes only; every
    // other epic completes through the cascade (plan/04).
    pub async fn complete_epic(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        epic: EpicId,
        reason: String,
    ) -> Result<CommandResult<Epic>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = epic_scope(tx, &project, &epic).await?;
                require_capability(&actor, Capability::ResolveWork)?;
                require_revision(closure_ctx.expected_revision, scope.epic.revision)?;
                scope.ensure_unarchived()?;
                if scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if scope.epic.status == EpicStatus::Proposed {
                    return Err(DomainError::InvalidState(
                        "epic is proposed and must be accepted first".into(),
                    ));
                }
                if scope.epic.block.is_some() {
                    return Err(DomainError::InvalidState("epic is blocked".into()));
                }
                let snapshot = workflow::eligibility::load_epic_snapshots(
                    tx,
                    &project,
                    &[epic],
                    closure_ctx.now,
                )
                .await?
                .remove(&epic)
                .ok_or_else(|| {
                    DomainError::from(StorageError::Corrupt(
                        "epic snapshot missing during completion checks".into(),
                    ))
                })?;
                if !snapshot
                    .epic_prereqs
                    .iter()
                    .all(|(_, status)| *status == EpicStatus::Done)
                {
                    return Err(DomainError::InvalidState(
                        "epic prerequisites are not all done".into(),
                    ));
                }
                if !workflow::epic::explicitly_completable(snapshot.task_counts) {
                    return Err(DomainError::InvalidState(
                        "epic has non-waived tasks; completion is automatic".into(),
                    ));
                }
                let reason = validate_required_text("reason", &reason, REASON_MAX_CHARS)?;
                let mut pending = Vec::new();
                workflow::epic::complete(tx, &closure_ctx, &scope.epic, &mut pending).await?;
                let dependents = workflow::dependents_of(tx, &[epic]).await?;
                workflow::recompute(
                    tx,
                    AffectedScope {
                        project,
                        epics: dependents,
                    },
                    &mut pending,
                    closure_ctx.now,
                )
                .await?;
                let events =
                    append_events(tx, &project, &closure_ctx, "completeEpic", &reason, pending)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::*;

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
async fn block_epic_revokes_descendant_claims_and_bumps_those_tasks() {
    let f = fixture().await;
    let s = lifecycle_scope(&f).await;
    let second = task(&f, s.project, s.epic.id, "Second").await;
    let first_claim = seed_claim_row(&f, s.task.id).await;
    let second_claim = seed_claim_row(&f, second.id).await;
    seed_submission_row(&f, s.task.id).await;
    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    let blocked = f
        .store
        .block_epic(
            ctx(&agent, &f.clock, Some(1)),
            s.project,
            s.epic.id,
            "rescoping".into(),
        )
        .await
        .unwrap();
    assert_eq!(blocked.value.block.expect("block").reason, "rescoping");
    assert_eq!(claim_state(&f, &first_claim).await.0, "revoked");
    assert_eq!(claim_state(&f, &second_claim).await.0, "revoked");
    // Claim-revoked tasks changed representation: one bump each (plan/04).
    assert_eq!(task_state(&f, s.task.id).await.2, 2);
    assert_eq!(task_state(&f, second.id).await.2, 2);
    let pending: String =
        sqlx::query_scalar("SELECT status FROM submissions WHERE task_id = ?1")
            .bind(s.task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(pending, "pending");
    let rows = event_rows(&f, &blocked.events).await;
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _)| kind == "task.changed")
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _)| kind == "claim.changed")
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _)| kind == "epic.changed")
            .count(),
        1
    );

    let err = f
        .store
        .block_epic(
            ctx(&f.owner, &f.clock, Some(2)),
            s.project,
            s.epic.id,
            "again".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));
}

#[tokio::test]
async fn unblock_epic_completes_children_finished_before_the_block() {
    let f = fixture().await;
    let s = lifecycle_scope(&f).await;
    f.store
        .block_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            s.project,
            s.epic.id,
            "hold".into(),
        )
        .await
        .unwrap();
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), s.project, s.task.id)
        .await
        .unwrap();
    // An explicitly blocked epic cannot complete (plan/04 FLOW-03).
    assert_eq!(epic_state(&f, s.epic.id).await.0, "open");

    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;
    let err = f
        .store
        .unblock_epic(ctx(&agent, &f.clock, Some(2)), s.project, s.epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    let unblocked = f
        .store
        .unblock_epic(ctx(&f.owner, &f.clock, Some(2)), s.project, s.epic.id)
        .await
        .unwrap();
    // Unblock re-evaluates children: done in the same command, one event.
    assert_eq!(epic_state(&f, s.epic.id).await, ("done".into(), 3));
    let rows = event_rows(&f, &unblocked.events).await;
    let epic_events: Vec<_> = rows
        .iter()
        .filter(|(kind, _, _, _, _)| kind == "epic.changed")
        .collect();
    assert_eq!(epic_events.len(), 1);
    assert_eq!(epic_events[0].2, 3);
    assert!(
        rows.iter()
            .all(|(_, _, _, action, reason)| action == "unblockEpic" && reason == "hold")
    );

    let err = f
        .store
        .unblock_epic(ctx(&f.owner, &f.clock, Some(3)), s.project, s.epic.id)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));
}

#[tokio::test]
async fn cancel_epic_preserves_done_descendants_and_propagates_the_reason() {
    let f = fixture().await;
    let s = lifecycle_scope(&f).await;
    let done = task(&f, s.project, s.epic.id, "Done work").await;
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), s.project, done.id)
        .await
        .unwrap();
    let claim = seed_claim_row(&f, s.task.id).await;
    let submission = seed_submission_row(&f, s.task.id).await;

    let cancelled = f
        .store
        .cancel_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            s.project,
            s.epic.id,
            "descoped".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        cancelled.value.cancellation.expect("record").reason,
        "descoped"
    );
    assert_eq!(epic_state(&f, s.epic.id).await, ("cancelled".into(), 2));
    // Done descendants stay done and unbumped; nonterminal ones cancel
    // with the propagated reason.
    assert_eq!(
        task_state(&f, done.id).await,
        ("done".into(), "complete".into(), 2)
    );
    assert_eq!(
        task_state(&f, s.task.id).await,
        ("cancelled".into(), "complete".into(), 2)
    );
    let reason: String =
        sqlx::query_scalar("SELECT cancellation_reason FROM tasks WHERE id = ?1")
            .bind(s.task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
    assert_eq!(reason, "descoped");
    assert_eq!(claim_state(&f, &claim).await.0, "revoked");
    let status: String = sqlx::query_scalar("SELECT status FROM submissions WHERE id = ?1")
        .bind(&submission)
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    assert_eq!(status, "withdrawn");
    let rows = event_rows(&f, &cancelled.events).await;
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _)| kind == "task.changed")
            .count(),
        1
    );
}

#[tokio::test]
async fn cancel_epic_bumps_done_tasks_whose_work_was_settled() {
    let f = fixture().await;
    let s = lifecycle_scope(&f).await;
    let done = task(&f, s.project, s.epic.id, "Done work").await;
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), s.project, done.id)
        .await
        .unwrap();
    // Fixture-only today (claims settle with reports in step 010), but the
    // preserved done task still changes representation when its lease and
    // pending submission are settled — same bump rule as blockEpic.
    seed_claim_row(&f, done.id).await;
    seed_submission_row(&f, done.id).await;

    let cancelled = f
        .store
        .cancel_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            s.project,
            s.epic.id,
            "descoped".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        task_state(&f, done.id).await,
        ("done".into(), "complete".into(), 3)
    );
    let rows = event_rows(&f, &cancelled.events).await;
    let done_events: Vec<_> = rows
        .iter()
        .filter(|(kind, id, _, _, _)| kind == "task.changed" && *id == done.id.to_string())
        .collect();
    assert_eq!(done_events.len(), 1);
    assert_eq!(done_events[0].2, 3);
}

#[tokio::test]
async fn complete_epic_rejects_proposed_blocked_gated_and_automatic_shapes() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let g = goal(&f, p.id, "G").await;
    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    // Proposed: agent creation under the default proposal gate.
    let proposed = f
        .store
        .create_epic(
            ctx(&agent, &f.clock, None),
            p.id,
            g.id,
            EpicCreate {
                title: "Proposed".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let err = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            proposed.id,
            "done".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));

    // Blocked.
    let blocked = epic(&f, p.id, g.id, "Blocked").await;
    f.store
        .block_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            blocked.id,
            "hold".into(),
        )
        .await
        .unwrap();
    let err = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(2)),
            p.id,
            blocked.id,
            "done".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));

    // Unmet epic prerequisite.
    let prereq = epic(&f, p.id, g.id, "Prereq").await;
    task(&f, p.id, prereq.id, "Keeps it open").await;
    let gated = epic(&f, p.id, g.id, "Gated").await;
    f.store
        .create_dependency(
            ctx(&f.owner, &f.clock, None),
            p.id,
            DependencyCreate::Epic {
                dependent_id: gated.id,
                prerequisite_id: prereq.id,
            },
        )
        .await
        .unwrap();
    let err = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(2)),
            p.id,
            gated.id,
            "done".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));

    // Non-waived tasks: completion is automatic, not explicit.
    let working = epic(&f, p.id, g.id, "Working").await;
    task(&f, p.id, working.id, "Open work").await;
    let err = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            working.id,
            "done".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidState(_)));

    // Agents never complete epics.
    let empty = epic(&f, p.id, g.id, "Empty").await;
    let err = f
        .store
        .complete_epic(
            ctx(&agent, &f.clock, Some(1)),
            p.id,
            empty.id,
            "done".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn complete_epic_covers_empty_and_all_waived_and_cascades_downstream() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let g = goal(&f, p.id, "G").await;

    // Empty epic gating a finished dependent.
    let empty = epic(&f, p.id, g.id, "Empty").await;
    let dependent = epic(&f, p.id, g.id, "Dependent").await;
    let dependent_task = task(&f, p.id, dependent.id, "Work").await;
    f.store
        .create_dependency(
            ctx(&f.owner, &f.clock, None),
            p.id,
            DependencyCreate::Epic {
                dependent_id: dependent.id,
                prerequisite_id: empty.id,
            },
        )
        .await
        .unwrap();
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), p.id, dependent_task.id)
        .await
        .unwrap();
    assert_eq!(epic_state(&f, dependent.id).await.0, "open");

    let completed = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(2)),
            p.id,
            empty.id,
            "nothing to do".into(),
        )
        .await
        .unwrap();
    assert_eq!(completed.value.status, EpicStatus::Done);
    // Downstream cascade in the same command.
    assert_eq!(epic_state(&f, dependent.id).await.0, "done");
    let rows = event_rows(&f, &completed.events).await;
    assert_eq!(
        rows.iter()
            .filter(|(kind, _, _, _, _)| kind == "epic.changed")
            .count(),
        2
    );
    assert!(
        rows.iter()
            .all(|(_, _, _, action, _)| action == "completeEpic")
    );

    // All-waived epic completes explicitly, never automatically.
    let waived_epic = epic(&f, p.id, g.id, "Waived").await;
    let only = task(&f, p.id, waived_epic.id, "Only").await;
    f.store
        .cancel_task(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            only.id,
            "obsolete".into(),
        )
        .await
        .unwrap();
    f.store
        .waive_task(
            ctx(&f.owner, &f.clock, Some(2)),
            p.id,
            only.id,
            "not needed".into(),
        )
        .await
        .unwrap();
    assert_eq!(epic_state(&f, waived_epic.id).await.0, "open");
    let completed = f
        .store
        .complete_epic(
            ctx(&f.owner, &f.clock, Some(2)),
            p.id,
            waived_epic.id,
            "all waived".into(),
        )
        .await
        .unwrap();
    assert_eq!(completed.value.status, EpicStatus::Done);
}

#[tokio::test]
async fn accept_epic_cascade_completes_already_done_children() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let g = goal(&f, p.id, "G").await;
    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;
    let proposed = f
        .store
        .create_epic(
            ctx(&agent, &f.clock, None),
            p.id,
            g.id,
            EpicCreate {
                title: "Proposed".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value;
    let t = task(&f, p.id, proposed.id, "T").await;
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), p.id, t.id)
        .await
        .unwrap();
    // A proposed epic never auto-completes (plan/04).
    assert_eq!(epic_state(&f, proposed.id).await.0, "proposed");

    let accepted = f
        .store
        .accept_epic(ctx(&f.owner, &f.clock, Some(1)), p.id, proposed.id)
        .await
        .unwrap();
    // Accept runs the cascade: open then done in one command, one event.
    assert_eq!(epic_state(&f, proposed.id).await, ("done".into(), 2));
    let rows = event_rows(&f, &accepted.events).await;
    let epic_events: Vec<_> = rows
        .iter()
        .filter(|(kind, _, _, _, _)| kind == "epic.changed")
        .collect();
    assert_eq!(epic_events.len(), 1);
    assert_eq!(epic_events[0].2, 2);
}

#[tokio::test]
async fn update_epic_rejects_empty_patch() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let g = goal(&f, p.id, "G").await;
    let e = epic(&f, p.id, g.id, "E").await;

    let err = f
        .store
        .update_epic(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            e.id,
            TextPatch::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "patch", .. }
    ));
}

#[tokio::test]
async fn create_epic_validates_description_length() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let g = goal(&f, p.id, "G").await;

    let err = f
        .store
        .create_epic(
            ctx(&f.owner, &f.clock, None),
            p.id,
            g.id,
            EpicCreate {
                title: "E".to_string(),
                description: Some("d".repeat(DESCRIPTION_MAX_CHARS + 1)),
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
