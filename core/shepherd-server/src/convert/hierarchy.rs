use shepherd_core::model as core;

use crate::generated::types as wire;

pub(crate) fn review_policy(policy: core::ReviewPolicy) -> wire::ReviewPolicy {
    match policy {
        core::ReviewPolicy::Human => wire::ReviewPolicy::Human,
        core::ReviewPolicy::Agent => wire::ReviewPolicy::Agent,
        core::ReviewPolicy::None => wire::ReviewPolicy::None,
    }
}

pub(crate) fn core_review_policy(policy: wire::ReviewPolicy) -> core::ReviewPolicy {
    match policy {
        wire::ReviewPolicy::Human => core::ReviewPolicy::Human,
        wire::ReviewPolicy::Agent => core::ReviewPolicy::Agent,
        wire::ReviewPolicy::None => core::ReviewPolicy::None,
    }
}

pub(crate) fn counts(counts: &core::Counts) -> wire::Counts {
    wire::Counts {
        total: counts.total as i32,
        done: counts.done as i32,
        cancelled: counts.cancelled as i32,
        waived: counts.waived as i32,
    }
}

/// Core keeps at most one lifecycle record; the wire shape is a 0/1-element
/// array (contract maxItems: 1).
pub(crate) fn lifecycle(record: &Option<core::LifecycleRecord>) -> Vec<wire::LifecycleRecord> {
    record
        .iter()
        .map(|record| wire::LifecycleRecord {
            actor_id: record.actor_id.as_uuid(),
            reason: record.reason.clone(),
            created_at: record.created_at,
        })
        .collect()
}

pub(crate) fn settings(settings: core::ProjectSettings) -> wire::ProjectSettings {
    wire::ProjectSettings {
        proposal_gate: settings.proposal_gate,
        planning_required: settings.planning_required,
        plan_review: review_policy(settings.plan_review),
        work_review: review_policy(settings.work_review),
    }
}

pub(crate) fn core_settings(settings: wire::ProjectSettings) -> core::ProjectSettings {
    core::ProjectSettings {
        proposal_gate: settings.proposal_gate,
        planning_required: settings.planning_required,
        plan_review: core_review_policy(settings.plan_review),
        work_review: core_review_policy(settings.work_review),
    }
}

pub(crate) fn project(project: core::Project) -> wire::Project {
    wire::Project {
        id: project.id.as_uuid(),
        revision: project.revision.value() as i32,
        created_at: project.created_at,
        updated_at: project.updated_at,
        name: project.name,
        description: project.description,
        settings: settings(project.settings),
        archived: project.archived,
        epic_counts: counts(&project.epic_counts),
        archives: lifecycle(&project.archive),
    }
}

pub(crate) fn goal(goal: core::Goal) -> wire::Goal {
    wire::Goal {
        id: goal.id.as_uuid(),
        revision: goal.revision.value() as i32,
        created_at: goal.created_at,
        updated_at: goal.updated_at,
        project_id: goal.project_id.as_uuid(),
        title: goal.title,
        description: goal.description,
        archived: goal.archived,
        completed: goal.completed,
        epic_counts: counts(&goal.epic_counts),
        archives: lifecycle(&goal.archive),
    }
}

pub(crate) fn task_type(task_type: core::TaskType) -> wire::TaskType {
    wire::TaskType {
        id: task_type.id.as_uuid(),
        revision: task_type.revision.value() as i32,
        created_at: task_type.created_at,
        updated_at: task_type.updated_at,
        project_id: task_type.project_id.as_uuid(),
        key: task_type.key,
        label: task_type.label,
        archived: task_type.archived,
        builtin: task_type.builtin,
    }
}

pub(crate) fn core_project_create(input: wire::ProjectCreate) -> core::ProjectCreate {
    core::ProjectCreate {
        name: input.name,
        description: input.description,
        settings: input.settings.map(core_settings),
    }
}

pub(crate) fn core_project_patch(input: wire::ProjectPatch) -> core::ProjectPatch {
    core::ProjectPatch {
        name: input.name,
        description: input.description,
        settings: input.settings.map(core_settings),
    }
}

pub(crate) fn core_goal_create(input: wire::GoalCreate) -> core::GoalCreate {
    core::GoalCreate {
        title: input.title,
        description: input.description,
    }
}

pub(crate) fn core_text_patch(input: wire::TextPatch) -> core::TextPatch {
    core::TextPatch {
        title: input.title,
        description: input.description,
    }
}

pub(crate) fn core_task_type_create(input: wire::TaskTypeCreate) -> core::TaskTypeCreate {
    core::TaskTypeCreate {
        key: input.key,
        label: input.label,
    }
}

pub(crate) fn core_task_type_patch(input: wire::TaskTypePatch) -> core::TaskTypePatch {
    core::TaskTypePatch {
        label: input.label,
        archived: input.archived,
    }
}
