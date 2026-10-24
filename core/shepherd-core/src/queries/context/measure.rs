// Budget measurement and deterministic fill for the context bundle. Step 015
// converts ContextBundle to the wire shape; the field sets measured here must
// stay identical to that conversion.
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{Counts, Epic, Goal, LifecycleRecord, Project, Task};
use crate::storage::StorageError;
use crate::storage::rows::format_ts;

/// Response budget in UTF-8 JSON bytes (plan/05): 256 KiB.
pub const CONTEXT_BUDGET_BYTES: usize = 262_144;

pub(super) fn measured<T: Serialize>(item: &T) -> Result<usize, DomainError> {
    Ok(serde_json::to_vec(item)
        .map_err(|err| StorageError::Corrupt(format!("context item: {err}")))?
        .len())
}

fn lifecycle_json(record: Option<&LifecycleRecord>) -> Value {
    match record {
        Some(record) => json!([{
            "actor_id": record.actor_id.to_string(),
            "reason": record.reason,
            "created_at": format_ts(&record.created_at),
        }]),
        None => json!([]),
    }
}

fn counts_json(counts: &Counts) -> Value {
    json!({
        "total": counts.total,
        "done": counts.done,
        "cancelled": counts.cancelled,
        "waived": counts.waived,
    })
}

// Eligibility is actor-relative and computed at wire conversion (step 015);
// the budget measures its fixed skeleton and the handoff records the delta.
fn eligibility_json() -> Value {
    json!({
        "can_plan": true,
        "can_execute": true,
        "can_review": true,
        "reasons": [],
        "allowed_actions": [],
    })
}

fn uuid_strings(ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(Uuid::to_string).collect()
}

// Wire-shaped mandatory entities (core Task/Epic/Goal/Project carry no serde);
// step 015's conversion must keep these field sets identical.
pub(super) fn mandatory_json(
    bundle_task: &Task,
    epic: &Epic,
    goal: &Goal,
    project: &Project,
    selected_plan_revision_ids: &[Uuid],
    accepted_plan_submission_ids: &[Uuid],
) -> Value {
    let task = json!({
        "id": bundle_task.id.to_string(),
        "revision": bundle_task.revision.value(),
        "created_at": format_ts(&bundle_task.created_at),
        "updated_at": format_ts(&bundle_task.updated_at),
        "project_id": bundle_task.project_id.to_string(),
        "epic_id": bundle_task.epic_id.to_string(),
        "title": bundle_task.title,
        "description": bundle_task.description,
        "type_key": bundle_task.type_key,
        "status": bundle_task.status.as_str(),
        "phase": bundle_task.phase.as_str(),
        "planning_required": bundle_task.planning_required,
        "plan_review": bundle_task.plan_review.as_str(),
        "work_review": bundle_task.work_review.as_str(),
        "selected_plan_revision_ids": uuid_strings(selected_plan_revision_ids),
        "accepted_plan_submission_ids": uuid_strings(accepted_plan_submission_ids),
        "archived": bundle_task.archived,
        "blocks": lifecycle_json(bundle_task.block.as_ref()),
        "waivers": lifecycle_json(bundle_task.waiver.as_ref()),
        "attempt_count": bundle_task.attempt_count,
        "eligibility": eligibility_json(),
        "cancellations": lifecycle_json(bundle_task.cancellation.as_ref()),
        "archives": lifecycle_json(bundle_task.archive.as_ref()),
    });
    let epic = json!({
        "id": epic.id.to_string(),
        "revision": epic.revision.value(),
        "created_at": format_ts(&epic.created_at),
        "updated_at": format_ts(&epic.updated_at),
        "project_id": epic.project_id.to_string(),
        "goal_id": epic.goal_id.to_string(),
        "title": epic.title,
        "description": epic.description,
        "status": epic.status.as_str(),
        "archived": epic.archived,
        "blocks": lifecycle_json(epic.block.as_ref()),
        "eligibility": eligibility_json(),
        "task_counts": counts_json(&epic.task_counts),
        "cancellations": lifecycle_json(epic.cancellation.as_ref()),
        "archives": lifecycle_json(epic.archive.as_ref()),
    });
    let goal = json!({
        "id": goal.id.to_string(),
        "revision": goal.revision.value(),
        "created_at": format_ts(&goal.created_at),
        "updated_at": format_ts(&goal.updated_at),
        "project_id": goal.project_id.to_string(),
        "title": goal.title,
        "description": goal.description,
        "archived": goal.archived,
        "epic_counts": counts_json(&goal.epic_counts),
        "completed": goal.completed,
        "archives": lifecycle_json(goal.archive.as_ref()),
    });
    let project = json!({
        "id": project.id.to_string(),
        "revision": project.revision.value(),
        "created_at": format_ts(&project.created_at),
        "updated_at": format_ts(&project.updated_at),
        "name": project.name,
        "description": project.description,
        "settings": {
            "proposal_gate": project.settings.proposal_gate,
            "planning_required": project.settings.planning_required,
            "plan_review": project.settings.plan_review.as_str(),
            "work_review": project.settings.work_review.as_str(),
        },
        "archived": project.archived,
        "epic_counts": counts_json(&project.epic_counts),
        "archives": lifecycle_json(project.archive.as_ref()),
    });
    // Full bundle skeleton: section keys, flag and paths cost bytes too.
    json!({
        "task": task,
        "epic": epic,
        "goal": goal,
        "project": project,
        "selected_plan": [],
        "documents": [],
        "submissions": [],
        "reviews": [],
        "sessions": [],
        "dependencies": [],
        "sibling_claims": [],
        "truncated": false,
        "continuation_paths": [],
    })
}

// Deterministic item-level fill in fixed section priority: the first item that
// does not fit closes its section and every lower-priority one (plan/05).
pub(super) struct BudgetFill {
    pub(super) remaining: usize,
    pub(super) open: bool,
    pub(super) truncated: bool,
    pub(super) continuation_paths: Vec<String>,
}

impl BudgetFill {
    pub(super) fn take<T: Serialize>(
        &mut self,
        rows: Vec<T>,
        cap: usize,
        paths: &[String],
    ) -> Result<Vec<T>, DomainError> {
        let mut rows = rows;
        let mut cut = rows.len() > cap;
        rows.truncate(cap);
        let mut kept = Vec::with_capacity(rows.len());
        for item in rows {
            if !self.open {
                cut = true;
                break;
            }
            let cost = measured(&item)? + 1;
            if cost > self.remaining {
                self.open = false;
                cut = true;
                break;
            }
            self.remaining -= cost;
            kept.push(item);
        }
        if cut {
            self.truncated = true;
            for path in paths {
                if !self.continuation_paths.contains(path) {
                    self.continuation_paths.push(path.clone());
                }
            }
        }
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_fill_cuts_items_then_sections_deterministically() {
        let path_a = vec!["/a".to_string()];
        let path_b = vec!["/b".to_string(), "/b2".to_string()];
        let mut fill = BudgetFill {
            remaining: 20,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };

        // Three 9-byte items (8 serialized + separator): the third exceeds the
        // 20-byte budget and closes every lower-priority section.
        let items = vec![
            "xxxxxx".to_string(),
            "yyyyyy".to_string(),
            "zzzzzz".to_string(),
        ];
        let kept = fill.take(items, 10, &path_a).unwrap();
        assert_eq!(kept.len(), 2);
        assert!(!fill.open);
        assert!(fill.truncated);
        let lower = fill.take(vec!["a".to_string()], 10, &path_b).unwrap();
        assert!(lower.is_empty());
        assert_eq!(fill.continuation_paths, vec!["/a", "/b", "/b2"]);

        // A count-cap cut keeps lower sections open and deduplicates paths.
        let mut fill = BudgetFill {
            remaining: 1_000,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };
        let kept = fill
            .take(vec!["a".to_string(), "b".to_string()], 1, &path_a)
            .unwrap();
        assert_eq!(kept, vec!["a".to_string()]);
        assert!(fill.open);
        let kept = fill
            .take(vec!["c".to_string(), "d".to_string()], 1, &path_a)
            .unwrap();
        assert_eq!(kept, vec!["c".to_string()]);
        assert_eq!(fill.continuation_paths, vec!["/a"]);
        assert!(fill.truncated);

        // Nothing to cut: no flag, no paths.
        let mut fill = BudgetFill {
            remaining: 1_000,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };
        fill.take(vec!["a".to_string()], 10, &path_a).unwrap();
        assert!(!fill.truncated);
        assert!(fill.continuation_paths.is_empty());
    }
}
