use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Counts, GoalId, LifecycleRecord, ProjectId, Revision, resource_status, typed_id_serde,
    typed_uuid,
};

typed_uuid!(EpicId);
typed_id_serde!(pub(super) serde_epic_id, EpicId);
resource_status!(EpicStatus);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Epic {
    #[serde(with = "serde_epic_id")]
    pub id: EpicId,
    #[serde(with = "super::content::serde_revision")]
    pub revision: Revision,
    #[serde(with = "super::claim::serde_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "super::claim::serde_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "super::content::serde_project_id")]
    pub project_id: ProjectId,
    #[serde(with = "super::goal::serde_goal_id")]
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
