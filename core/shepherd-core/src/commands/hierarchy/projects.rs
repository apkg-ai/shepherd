use super::*;

fn settings_json(settings: &ProjectSettings) -> Result<String, DomainError> {
    serde_json::to_string(settings)
        .map_err(|err| StorageError::Corrupt(format!("settings: {err}")).into())
}

impl Store {
    pub async fn create_project(
        &self,
        ctx: CommandContext,
        input: ProjectCreate,
    ) -> Result<CommandResult<Project>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 201, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                require_capability(&actor, Capability::AdministerProject)?;
                let name = validate_required_text("name", &input.name, NAME_MAX_CHARS)?;
                let description = input.description.unwrap_or_default();
                validate_long_text("description", &description, DESCRIPTION_MAX_CHARS)?;
                let settings = input.settings.unwrap_or_default();
                let id = ProjectId::generate(closure_ctx.now);
                let now_text = format_ts(&closure_ctx.now);
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
                    let type_id = TaskTypeId::generate(closure_ctx.now);
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

                let events =
                    append_events(tx, &id, &closure_ctx, "createProject", "", pending).await?;
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
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let current = project_row(tx, &project)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(closure_ctx.expected_revision, current.revision)?;
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
                .bind(format_ts(&closure_ctx.now))
                .bind(&name)
                .bind(&description)
                .bind(settings_json(&settings)?)
                .bind(project.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "updateProject",
                    "",
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
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
                    ..Default::default()
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
    async fn hierarchy_commands_replay_and_conflict_on_hash_change() {
        let f = fixture().await;
        let project = project(&f, "P").await;
        let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let mut ctx = ctx(&f.owner, &f.clock, None);
        ctx.idempotency_key = key;
        ctx.request_hash = Some("goal-hash-a".into());
        let input = GoalCreate {
            title: "G".to_string(),
            description: None,
        };
        let first = f
            .store
            .create_goal(ctx.clone(), project.id, input.clone())
            .await
            .unwrap();
        assert_eq!(first.events.len(), 1);

        let replay = f
            .store
            .create_goal(ctx.clone(), project.id, input)
            .await
            .unwrap();
        assert_eq!(
            replay.value.id, first.value.id,
            "replay is the sealed original"
        );
        assert_eq!(replay.value.revision, first.value.revision);
        assert_eq!(replay.events, first.events);

        // Same key, different request: rejected before any write.
        ctx.request_hash = Some("goal-hash-b".into());
        assert!(matches!(
            f.store
                .create_goal(
                    ctx,
                    project.id,
                    GoalCreate {
                        title: "G2".to_string(),
                        description: None,
                    }
                )
                .await,
            Err(DomainError::IdempotencyConflict)
        ));

        let listed = f
            .store
            .list_goals(&project.id, &crate::queries::ListParams::default())
            .await
            .unwrap();
        assert_eq!(listed.items.len(), 1, "no duplicate row was created");
    }
}
