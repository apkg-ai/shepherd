use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::project::ReviewPolicy;
use super::{EpicId, LifecycleRecord, ProjectId, Revision, typed_uuid};

typed_uuid!(TaskId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Proposed,
    Open,
    Active,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Proposed => "proposed",
            TaskStatus::Open => "open",
            TaskStatus::Active => "active",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(TaskStatus::Proposed),
            "open" => Some(TaskStatus::Open),
            "active" => Some(TaskStatus::Active),
            "done" => Some(TaskStatus::Done),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    Planning,
    PlanReview,
    Execution,
    WorkReview,
    Complete,
}

impl TaskPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskPhase::Planning => "planning",
            TaskPhase::PlanReview => "plan_review",
            TaskPhase::Execution => "execution",
            TaskPhase::WorkReview => "work_review",
            TaskPhase::Complete => "complete",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "planning" => Some(TaskPhase::Planning),
            "plan_review" => Some(TaskPhase::PlanReview),
            "execution" => Some(TaskPhase::Execution),
            "work_review" => Some(TaskPhase::WorkReview),
            "complete" => Some(TaskPhase::Complete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub revision: Revision,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_id: ProjectId,
    pub epic_id: EpicId,
    pub title: String,
    pub description: String,
    pub type_key: String,
    pub status: TaskStatus,
    pub phase: TaskPhase,
    pub planning_required: bool,
    pub plan_review: ReviewPolicy,
    pub work_review: ReviewPolicy,
    pub archived: bool,
    pub attempt_count: i64,
    pub block: Option<LifecycleRecord>,
    pub waiver: Option<LifecycleRecord>,
    pub archive: Option<LifecycleRecord>,
    pub cancellation: Option<LifecycleRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskCreate {
    pub title: String,
    pub description: Option<String>,
    pub type_key: String,
    pub planning_required: Option<bool>,
    pub plan_review: Option<ReviewPolicy>,
    pub work_review: Option<ReviewPolicy>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub type_key: Option<String>,
    pub planning_required: Option<bool>,
    pub plan_review: Option<ReviewPolicy>,
    pub work_review: Option<ReviewPolicy>,
}

impl TaskPatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.type_key.is_none()
            && self.planning_required.is_none()
            && self.plan_review.is_none()
            && self.work_review.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_round_trips_stored_strings() {
        for status in [
            TaskStatus::Proposed,
            TaskStatus::Open,
            TaskStatus::Active,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TaskStatus::parse("blocked"), None);
    }

    #[test]
    fn terminal_statuses_are_done_and_cancelled() {
        assert!(!TaskStatus::Proposed.is_terminal());
        assert!(!TaskStatus::Open.is_terminal());
        assert!(!TaskStatus::Active.is_terminal());
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn task_phase_round_trips_stored_strings() {
        for phase in [
            TaskPhase::Planning,
            TaskPhase::PlanReview,
            TaskPhase::Execution,
            TaskPhase::WorkReview,
            TaskPhase::Complete,
        ] {
            assert_eq!(TaskPhase::parse(phase.as_str()), Some(phase));
        }
        assert_eq!(TaskPhase::parse("idle"), None);
    }

    #[test]
    fn task_patch_emptiness() {
        assert!(TaskPatch::default().is_empty());

        let title_only = TaskPatch {
            title: Some("New".into()),
            ..Default::default()
        };
        assert!(!title_only.is_empty());

        let desc_change = TaskPatch {
            description: Some("Changed".into()),
            ..Default::default()
        };
        assert!(!desc_change.is_empty());

        let policy_change = TaskPatch {
            plan_review: Some(ReviewPolicy::Agent),
            ..Default::default()
        };
        assert!(!policy_change.is_empty());
    }
}
