use super::*;

impl Store {
    pub async fn create_task_type(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        input: TaskTypeCreate,
    ) -> Result<CommandResult<TaskType>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 201, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope_archived = project_archived(tx, &project).await?;
                require_capability(&actor, Capability::AdministerProject)?;
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
                let id = TaskTypeId::generate(closure_ctx.now);
                sqlx::query(
                    "INSERT INTO task_types (id, revision, created_at, updated_at, project_id, \
                     key, label, archived, builtin) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 0, 0)",
                )
                .bind(id.to_string())
                .bind(Revision::INITIAL.value())
                .bind(format_ts(&closure_ctx.now))
                .bind(project.to_string())
                .bind(&key)
                .bind(&label)
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "createTaskType",
                    "",
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

    // Keys are immutable; this renames the label and archives (contract
    // TaskTypePatch). task_types has no lifecycle columns, archived is a flag.
    pub async fn update_task_type(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task_type: TaskTypeId,
        patch: TaskTypePatch,
    ) -> Result<CommandResult<TaskType>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let current = find_task_type(tx, &project, &task_type)
                    .await?
                    .ok_or(DomainError::NotFound)?;
                require_capability(&actor, Capability::AdministerProject)?;
                require_revision(closure_ctx.expected_revision, current.revision)?;
                if current.archived || project_archived(tx, &project).await? {
                    return Err(DomainError::ArchivedScope);
                }
                if patch.label.is_none() && patch.archived.is_none() {
                    return Err(DomainError::Validation {
                        field: "patch",
                        message: "at least one field must be provided".into(),
                    });
                }
                if patch.archived == Some(false) {
                    return Err(DomainError::Validation {
                        field: "archived",
                        message: "archive is one-way and cannot be cleared".into(),
                    });
                }
                let label = match &patch.label {
                    Some(value) => validate_type_label(value)?,
                    None => current.label,
                };
                let next = current.revision.next();
                sqlx::query(
                    "UPDATE task_types SET revision = ?1, updated_at = ?2, label = ?3, \
                     archived = ?4 WHERE id = ?5",
                )
                .bind(next.value())
                .bind(format_ts(&closure_ctx.now))
                .bind(&label)
                .bind(i64::from(patch.archived.unwrap_or(false)))
                .bind(task_type.to_string())
                .execute(&mut **tx)
                .await?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "updateTaskType",
                    "",
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
    use super::*;
    use super::super::test_support::*;

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
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(renamed.revision.value(), 2);
    assert_eq!(renamed.label, "Operations");
    assert_eq!(renamed.key, "ops");
    assert!(!renamed.archived);

    // Archived registry entries are frozen; the flag is one-way.
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

#[tokio::test]
async fn update_task_type_rejects_archive_reversal_and_validates_label() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let custom = f
        .store
        .create_task_type(
            ctx(&f.owner, &f.clock, None),
            p.id,
            TaskTypeCreate {
                key: "ops".to_string(),
                label: "Ops".to_string(),
            },
        )
        .await
        .unwrap()
        .value;

    // Clearing the archive flag is rejected: archival is one-way.
    let err = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            custom.id,
            TaskTypePatch {
                archived: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "archived",
            ..
        }
    ));

    // Blank label is rejected.
    let err = f
        .store
        .update_task_type(
            ctx(&f.owner, &f.clock, Some(1)),
            p.id,
            custom.id,
            TaskTypePatch {
                label: Some("   ".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation { field: "label", .. }
    ));
}

#[tokio::test]
async fn create_task_type_rejects_agents() {
    let f = fixture().await;
    let p = project(&f, "P").await;
    let agent = person(&f.clock, ActorKind::Agent, "agent");
    register(&f.store, &agent).await;

    let err = f
        .store
        .create_task_type(
            ctx(&agent, &f.clock, None),
            p.id,
            TaskTypeCreate {
                key: "ops".to_string(),
                label: "Ops".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}
}
