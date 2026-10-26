// Task plan selection (plan/03, plan/04 row "select plan").
use super::*;

impl Store {
    pub async fn select_task_plan(
        &self,
        ctx: CommandContext,
        project: ProjectId,
        task_id: TaskId,
        input: SelectPlanInput,
    ) -> Result<CommandResult<Task>, DomainError> {
        let closure_ctx = ctx.clone();
        self.idempotent_command(&ctx, 200, move |tx| {
            Box::pin(async move {
                let now_text = format_ts(&closure_ctx.now);

                // Authenticate before route membership (plan/04 precedence).
                let actor = live_actor(tx, &closure_ctx.actor.id).await?;
                let scope = task_scope(tx, &project, &task_id).await?;
                require_capability(&actor, Capability::AuthorDocuments)?;
                require_revision(closure_ctx.expected_revision, scope.task.revision)?;
                scope.ensure_unarchived()?;
                if scope.task.status.is_terminal() || scope.epic.status.is_terminal() {
                    return Err(DomainError::TerminalScope);
                }
                if task_execute_or_review_work(tx, task_id, &now_text).await? {
                    return Err(DomainError::ActiveWork(
                        "plan selection is blocked by an execute/review claim or a \
                         pending submission"
                            .into(),
                    ));
                }
                if !scope.task.planning_required {
                    return Err(DomainError::Validation {
                        field: "planning_required",
                        message: "task does not require planning".into(),
                    });
                }
                require_task_plan_revision(tx, &project, &task_id, &input.document_revision_id)
                    .await?;

                // Re-selecting the current revision changes nothing: acceptance
                // retained, no revision bump, no event (plan/04 select-plan row).
                if selected_plan_revision(tx, task_id).await? == Some(input.document_revision_id) {
                    return Ok(CommandResult {
                        value: scope.task,
                        events: Vec::new(),
                    });
                }

                sqlx::query(
                    "UPDATE tasks SET revision = revision + 1, updated_at = ?1, phase = ?2, \
                     selected_plan_revision_id = ?3, accepted_plan_submission_id = NULL \
                     WHERE id = ?4",
                )
                .bind(&now_text)
                .bind(TaskPhase::Planning.as_str())
                .bind(input.document_revision_id.to_string())
                .bind(task_id.to_string())
                .execute(&mut **tx)
                .await?;

                let updated = task_row(tx, &project, &task_id)
                    .await?
                    .ok_or_else(|| missing_after_write("task"))?;
                let events = append_events(
                    tx,
                    &project,
                    &closure_ctx,
                    "selectTaskPlan",
                    "",
                    vec![PendingEvent::task(task_id, updated.revision, scope.epic.id)],
                )
                .await?;
                Ok(CommandResult {
                    value: updated,
                    events,
                })
            })
        })
        .await
    }
}
