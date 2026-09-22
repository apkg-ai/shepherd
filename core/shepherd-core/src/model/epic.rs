use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{Counts, GoalId, LifecycleRecord, ProjectId, Revision, typed_uuid};

typed_uuid!(EpicId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpicStatus {
    Proposed,
    Open,
    Active,
    Done,
    Cancelled,
}

impl EpicStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EpicStatus::Proposed => "proposed",
            EpicStatus::Open => "open",
            EpicStatus::Active => "active",
            EpicStatus::Done => "done",
            EpicStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(EpicStatus::Proposed),
            "open" => Some(EpicStatus::Open),
            "active" => Some(EpicStatus::Active),
            "done" => Some(EpicStatus::Done),
            "cancelled" => Some(EpicStatus::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, EpicStatus::Done | EpicStatus::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Epic {
    pub id: EpicId,
    pub revision: Revision,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_id: ProjectId,
    pub goal_id: GoalId,
    pub title: String,
    pub description: String,
    pub status: EpicStatus,
    pub archived: bool,
    pub task_counts: Counts,
    pub block: Option<LifecycleRecord>,
    pub archive: Option<LifecycleRecord>,
    pub cancellation: Option<LifecycleRecord>,
}

#[derive(Debug, Clone)]
pub struct EpicCreate {
    pub title: String,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epic_status_round_trips_stored_strings() {
        for status in [
            EpicStatus::Proposed,
            EpicStatus::Open,
            EpicStatus::Active,
            EpicStatus::Done,
            EpicStatus::Cancelled,
        ] {
            assert_eq!(EpicStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(EpicStatus::parse("blocked"), None);
    }

    #[test]
    fn terminal_statuses_are_done_and_cancelled() {
        assert!(!EpicStatus::Proposed.is_terminal());
        assert!(!EpicStatus::Open.is_terminal());
        assert!(!EpicStatus::Active.is_terminal());
        assert!(EpicStatus::Done.is_terminal());
        assert!(EpicStatus::Cancelled.is_terminal());
    }
}
