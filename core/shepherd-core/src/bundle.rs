//! Context-bundle assembly.
//!
//! Pure function — takes preloaded data and assembles the bundle a caller
//! receives on task claim. The store handles the loading; this module handles
//! the assembly logic.

use crate::model::{
    AncestorSummary, ContextBundle, Identity, KnowledgeItem, Session, SiblingTask, Task,
};

/// Assemble a context bundle from preloaded data.
///
/// - `task`: the claimed task itself.
/// - `ancestors_with_sessions`: ancestor tasks (dependency chain + parent
///   chain) with their sessions, ordered root-first.
/// - `project_knowledge`: project-scoped knowledge items.
/// - `sibling_claims`: other in-progress tasks in the same project with their
///   claimant identity.
pub fn assemble(
    task: Task,
    ancestors_with_sessions: Vec<(Task, Vec<Session>)>,
    project_knowledge: Vec<KnowledgeItem>,
    sibling_claims: Vec<(Task, Identity)>,
) -> ContextBundle {
    // Build ancestor summaries from tasks and their sessions.
    let ancestor_summaries: Vec<AncestorSummary> = ancestors_with_sessions
        .iter()
        .map(|(ancestor, sessions)| {
            // Collect decisions from all sessions on this ancestor.
            let decisions: Vec<String> = sessions
                .iter()
                .flat_map(|s| s.decisions.iter().cloned())
                .collect();

            // Use the most recent successful session's summary, or fall back
            // to the task description.
            let summary = sessions
                .iter()
                .rev()
                .find(|s| s.outcome == crate::model::SessionOutcome::Succeeded)
                .map(|s| s.summary.clone())
                .unwrap_or_default();

            AncestorSummary {
                task_id: ancestor.id,
                title: ancestor.title.clone(),
                status: ancestor.status,
                summary,
                decisions,
            }
        })
        .collect();

    // Collect artifact URIs from the task itself and all ancestors.
    let mut artifacts: Vec<String> = Vec::new();
    for (_, sessions) in &ancestors_with_sessions {
        for session in sessions {
            artifacts.extend(session.artifacts.iter().cloned());
        }
    }

    // Build sibling task info.
    let sibling_tasks: Vec<SiblingTask> = sibling_claims
        .into_iter()
        .filter(|(t, _)| t.id != task.id)
        .map(|(t, identity)| SiblingTask {
            task_id: t.id,
            title: t.title.clone(),
            status: t.status,
            claimed_by: Some(identity),
        })
        .collect();

    ContextBundle {
        task,
        ancestor_summaries,
        project_knowledge,
        artifacts,
        sibling_tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::Utc;

    fn make_task(id: TaskId, title: &str) -> Task {
        Task {
            id,
            project_id: ProjectId::new(),
            title: title.into(),
            description: String::new(),
            task_type: TaskType::Code,
            status: TaskStatus::Done,
            metadata: serde_json::json!({}),
            assignee: None,
            graph_role: vec![],
            attempt_count: 0,
            blocked_from_status: None,
            block_reason: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_session(task_id: TaskId, outcome: SessionOutcome) -> Session {
        let id = Identity {
            harness: "test".into(),
            agent_model: "test".into(),
            session_id: "s1".into(),
            label: None,
        };
        Session {
            id: SessionId::new(),
            task_id,
            identity: id,
            started_at: Utc::now(),
            ended_at: Utc::now(),
            outcome,
            failure_reason: None,
            summary: "did things".into(),
            decisions: vec!["chose X".into()],
            knowledge_items: vec![],
            artifacts: vec!["https://example.com/pr/1".into()],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn assembles_with_ancestors() {
        let target = make_task(TaskId::new(), "target task");
        let ancestor = make_task(TaskId::new(), "ancestor");
        let session = make_session(ancestor.id, SessionOutcome::Succeeded);

        let bundle = assemble(
            target,
            vec![(ancestor.clone(), vec![session])],
            vec![],
            vec![],
        );

        assert_eq!(bundle.ancestor_summaries.len(), 1);
        assert_eq!(bundle.ancestor_summaries[0].title, "ancestor");
        assert_eq!(bundle.ancestor_summaries[0].summary, "did things");
        assert_eq!(bundle.ancestor_summaries[0].decisions, vec!["chose X"]);
        assert_eq!(bundle.artifacts, vec!["https://example.com/pr/1"]);
    }

    #[test]
    fn filters_self_from_siblings() {
        let target = make_task(TaskId::new(), "target");
        let sibling = make_task(TaskId::new(), "sibling");
        let identity = Identity {
            harness: "cc".into(),
            agent_model: "opus".into(),
            session_id: "s2".into(),
            label: None,
        };

        let bundle = assemble(
            target.clone(),
            vec![],
            vec![],
            vec![
                (target.clone(), identity.clone()),
                (sibling.clone(), identity),
            ],
        );

        assert_eq!(bundle.sibling_tasks.len(), 1);
        assert_eq!(bundle.sibling_tasks[0].title, "sibling");
    }
}
