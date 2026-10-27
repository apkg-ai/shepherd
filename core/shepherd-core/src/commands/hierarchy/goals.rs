use super::*;

impl Store {
    pub async fn create_goal(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: GoalCreate,
    ) -> Result<CommandResult<Goal>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 201, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope_archived = project_archived(tx, &project).await?;
                require_capability(&actor, Capability::AdministerProject)?;
                if scope_archived {
                    return Err(DomainError::ArchivedScope);
                }
                let title = validate_required_text("title", &input.title, NAME_MAX_CHARS)?;
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                let id = GoalId::generate(closure_ctx.now);
                sqlx::query(
                    "INSERT INTO goals (id, revision, created_at, updated_at, project_id, \
                     title, description, archived) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(format_ts(&closure_ctx.now))
                .bind(project.to_string())
                .bind(&title)
                .bind(&description)
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "createGoal",
                    "",
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
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                // Membership precedes the revision check: a foreign goal is 404, never 412.
                let current = goal_row(tx, &project, &goal)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(closure_ctx.expected_revision, current.revision)?;
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
                .bind(format_ts(&closure_ctx.now))
                .bind(&title)
                .bind(&description)
                .bind(goal.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "updateGoal",
                    "",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::*;

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
async fn create_goal_validates_title_and_description() {
    let f = fixture().await;
    let p = project(&f, "P").await;

    for title in ["   ", &"x".repeat(NAME_MAX_CHARS + 1)] {
        let err = f
            .store
            .create_goal(
                ctx(&f.owner, &f.clock, None),
                p.id,
                GoalCreate {
                    title: title.to_string(),
                    description: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "title", .. }
        ));
    }

    let err = f
        .store
        .create_goal(
            ctx(&f.owner, &f.clock, None),
            p.id,
            GoalCreate {
                title: "Valid".to_string(),
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

#[tokio::test]
async fn create_goal_rejects_agents() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    let err = f
        .store
        .create_goal(
            ctx(&agent, &f.clock, None),
            p.id,
            GoalCreate {
                title: "Goal".to_string(),
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}
}
