use shepherd_core::model as core;
use shepherd_core::queries::{EpicView, Graph, GraphNode, TaskView};
use shepherd_core::workflow::eligibility::{Eligibility, GateCode};

use crate::generated::server::api::ListDependenciesLevel;
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

pub(crate) fn entity_status(status: core::EntityStatus) -> wire::EntityStatus {
    match status {
        core::EntityStatus::Proposed => wire::EntityStatus::Proposed,
        core::EntityStatus::Open => wire::EntityStatus::Open,
        core::EntityStatus::Active => wire::EntityStatus::Active,
        core::EntityStatus::Done => wire::EntityStatus::Done,
        core::EntityStatus::Cancelled => wire::EntityStatus::Cancelled,
    }
}

pub(crate) fn core_epic_status(status: wire::EntityStatus) -> core::EpicStatus {
    match status {
        wire::EntityStatus::Proposed => core::EpicStatus::Proposed,
        wire::EntityStatus::Open => core::EpicStatus::Open,
        wire::EntityStatus::Active => core::EpicStatus::Active,
        wire::EntityStatus::Done => core::EpicStatus::Done,
        wire::EntityStatus::Cancelled => core::EpicStatus::Cancelled,
    }
}

pub(crate) fn core_task_status(status: wire::EntityStatus) -> core::TaskStatus {
    match status {
        wire::EntityStatus::Proposed => core::TaskStatus::Proposed,
        wire::EntityStatus::Open => core::TaskStatus::Open,
        wire::EntityStatus::Active => core::TaskStatus::Active,
        wire::EntityStatus::Done => core::TaskStatus::Done,
        wire::EntityStatus::Cancelled => core::TaskStatus::Cancelled,
    }
}

pub(crate) fn task_phase(phase: core::TaskPhase) -> wire::TaskPhase {
    match phase {
        core::TaskPhase::Planning => wire::TaskPhase::Planning,
        core::TaskPhase::PlanReview => wire::TaskPhase::PlanReview,
        core::TaskPhase::Execution => wire::TaskPhase::Execution,
        core::TaskPhase::WorkReview => wire::TaskPhase::WorkReview,
        core::TaskPhase::Complete => wire::TaskPhase::Complete,
    }
}

pub(crate) fn core_task_phase(phase: wire::TaskPhase) -> core::TaskPhase {
    match phase {
        wire::TaskPhase::Planning => core::TaskPhase::Planning,
        wire::TaskPhase::PlanReview => core::TaskPhase::PlanReview,
        wire::TaskPhase::Execution => core::TaskPhase::Execution,
        wire::TaskPhase::WorkReview => core::TaskPhase::WorkReview,
        wire::TaskPhase::Complete => core::TaskPhase::Complete,
    }
}

fn gate_code(code: GateCode) -> wire::GateReasonCode {
    match code {
        GateCode::ProposalRequired => wire::GateReasonCode::ProposalRequired,
        GateCode::EpicProposalRequired => wire::GateReasonCode::EpicProposalRequired,
        GateCode::ExplicitBlock => wire::GateReasonCode::ExplicitBlock,
        GateCode::EpicBlock => wire::GateReasonCode::EpicBlock,
        GateCode::EpicPrerequisite => wire::GateReasonCode::EpicPrerequisite,
        GateCode::TaskPrerequisite => wire::GateReasonCode::TaskPrerequisite,
        GateCode::PlanRequired => wire::GateReasonCode::PlanRequired,
        GateCode::PlanReviewRequired => wire::GateReasonCode::PlanReviewRequired,
        GateCode::WorkReviewRequired => wire::GateReasonCode::WorkReviewRequired,
        GateCode::Claimed => wire::GateReasonCode::Claimed,
        GateCode::Terminal => wire::GateReasonCode::Terminal,
        GateCode::Archived => wire::GateReasonCode::Archived,
        GateCode::WrongPhase => wire::GateReasonCode::WrongPhase,
        GateCode::ReviewerPolicy => wire::GateReasonCode::ReviewerPolicy,
        GateCode::ProducerCannotReview => wire::GateReasonCode::ProducerCannotReview,
    }
}

pub(crate) fn eligibility(eligibility: Eligibility) -> wire::Eligibility {
    wire::Eligibility {
        can_plan: eligibility.can_plan,
        can_execute: eligibility.can_execute,
        can_review: eligibility.can_review,
        reasons: eligibility
            .reasons
            .into_iter()
            .map(|reason| wire::GateReason {
                code: gate_code(reason.code),
                resource_id: reason.resource_id,
                message: reason.message,
            })
            .collect(),
        allowed_actions: eligibility
            .allowed_actions
            .iter()
            .map(|action| action.to_string())
            .collect(),
    }
}

// Block and Waiver share LifecycleRecord's fields but are distinct wire types.
fn blocks(record: &Option<core::LifecycleRecord>) -> Vec<wire::Block> {
    record
        .iter()
        .map(|record| wire::Block {
            actor_id: record.actor_id.as_uuid(),
            reason: record.reason.clone(),
            created_at: record.created_at,
        })
        .collect()
}

fn waivers(record: &Option<core::LifecycleRecord>) -> Vec<wire::Waiver> {
    record
        .iter()
        .map(|record| wire::Waiver {
            actor_id: record.actor_id.as_uuid(),
            reason: record.reason.clone(),
            created_at: record.created_at,
        })
        .collect()
}

pub(crate) fn epic(view: EpicView) -> wire::Epic {
    let value = view.epic;
    wire::Epic {
        id: value.id.as_uuid(),
        revision: value.revision.value() as i32,
        created_at: value.created_at,
        updated_at: value.updated_at,
        project_id: value.project_id.as_uuid(),
        goal_id: value.goal_id.as_uuid(),
        title: value.title,
        description: value.description,
        status: entity_status(value.status.into()),
        archived: value.archived,
        task_counts: counts(&value.task_counts),
        eligibility: eligibility(view.eligibility),
        blocks: blocks(&value.block),
        cancellations: lifecycle(&value.cancellation),
        archives: lifecycle(&value.archive),
    }
}

pub(crate) fn task(view: TaskView) -> wire::Task {
    let value = view.task;
    wire::Task {
        id: value.id.as_uuid(),
        revision: value.revision.value() as i32,
        created_at: value.created_at,
        updated_at: value.updated_at,
        project_id: value.project_id.as_uuid(),
        epic_id: value.epic_id.as_uuid(),
        title: value.title,
        description: value.description,
        type_key: value.type_key,
        status: entity_status(value.status.into()),
        phase: task_phase(value.phase),
        planning_required: value.planning_required,
        plan_review: review_policy(value.plan_review),
        work_review: review_policy(value.work_review),
        selected_plan_revision_ids: view
            .selected_plan_revision_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        accepted_plan_submission_ids: view
            .accepted_plan_submission_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        archived: value.archived,
        attempt_count: value.attempt_count as i32,
        eligibility: eligibility(view.eligibility),
        blocks: blocks(&value.block),
        waivers: waivers(&value.waiver),
        cancellations: lifecycle(&value.cancellation),
        archives: lifecycle(&value.archive),
    }
}

pub(crate) fn dependency_level(level: core::DependencyLevel) -> wire::DependencyLevel {
    match level {
        core::DependencyLevel::Epic => wire::DependencyLevel::Epic,
        core::DependencyLevel::Task => wire::DependencyLevel::Task,
    }
}

pub(crate) fn core_dependency_level(level: ListDependenciesLevel) -> core::DependencyLevel {
    match level {
        ListDependenciesLevel::Epic => core::DependencyLevel::Epic,
        ListDependenciesLevel::Task => core::DependencyLevel::Task,
    }
}

pub(crate) fn dependency(value: core::Dependency) -> wire::Dependency {
    wire::Dependency {
        id: value.id.as_uuid(),
        revision: value.revision.value() as i32,
        created_at: value.created_at,
        updated_at: value.updated_at,
        project_id: value.project_id.as_uuid(),
        level: dependency_level(value.level),
        dependent_id: value.dependent_id,
        prerequisite_id: value.prerequisite_id,
    }
}

pub(crate) fn core_dependency_create(input: wire::DependencyCreate) -> core::DependencyCreate {
    match input.level {
        wire::DependencyCreateLevel::Epic => core::DependencyCreate::Epic {
            dependent_id: core::EpicId::from_uuid(input.dependent_id),
            prerequisite_id: core::EpicId::from_uuid(input.prerequisite_id),
        },
        wire::DependencyCreateLevel::Task => core::DependencyCreate::Task {
            dependent_id: core::TaskId::from_uuid(input.dependent_id),
            prerequisite_id: core::TaskId::from_uuid(input.prerequisite_id),
        },
    }
}

pub(crate) fn graph(value: Graph) -> wire::Graph {
    wire::Graph {
        level: match value.level {
            core::DependencyLevel::Epic => wire::GraphLevel::Epic,
            core::DependencyLevel::Task => wire::GraphLevel::Task,
        },
        scope_id: value.scope_id,
        nodes: value.nodes.into_iter().map(graph_node).collect(),
        dependencies: value.dependencies.into_iter().map(dependency).collect(),
    }
}

fn graph_node(node: GraphNode) -> wire::GraphNodesItem {
    wire::GraphNodesItem {
        id: node.id,
        title: node.title,
        status: entity_status(node.status),
        eligibility: eligibility(node.eligibility),
        counts: counts(&node.counts),
    }
}

/// The wire body carries the owning goal; core takes it as a route-style argument.
pub(crate) fn core_epic_create(input: wire::EpicCreate) -> (core::GoalId, core::EpicCreate) {
    (
        core::GoalId::from_uuid(input.goal_id),
        core::EpicCreate {
            title: input.title,
            description: input.description,
        },
    )
}

/// The wire body carries the owning epic; core takes it as a route-style argument.
pub(crate) fn core_task_create(input: wire::TaskCreate) -> (core::EpicId, core::TaskCreate) {
    (
        core::EpicId::from_uuid(input.epic_id),
        core::TaskCreate {
            title: input.title,
            description: input.description,
            type_key: input.type_key,
            planning_required: input.planning_required,
            plan_review: input.plan_review.map(core_review_policy),
            work_review: input.work_review.map(core_review_policy),
        },
    )
}

pub(crate) fn core_task_patch(input: wire::TaskPatch) -> core::TaskPatch {
    core::TaskPatch {
        title: input.title,
        description: input.description,
        type_key: input.type_key,
        planning_required: input.planning_required,
        plan_review: input.plan_review.map(core_review_policy),
        work_review: input.work_review.map(core_review_policy),
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
