use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Counts, LifecycleRecord, ProjectId, Revision, typed_id_serde, typed_uuid};

typed_uuid!(GoalId);
typed_id_serde!(pub(super) serde_goal_id, GoalId);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    #[serde(with = "serde_goal_id")]
    pub id: GoalId,
    #[serde(with = "super::content::serde_revision")]
    pub revision: Revision,
    #[serde(with = "super::claim::serde_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "super::claim::serde_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "super::content::serde_project_id")]
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub archived: bool,
    pub epic_counts: Counts,
    pub completed: bool,
    pub archive: Option<LifecycleRecord>,
}

#[derive(Debug, Clone)]
pub struct GoalCreate {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TextPatch {
    pub title: Option<String>,
    pub description: Option<String>,
}

// Derived, never stored (plan/03): empty goals are never complete.
pub fn goal_completed(counts: &Counts) -> bool {
    counts.total > 0 && counts.done == counts.total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_goal_is_not_completed() {
        assert!(!goal_completed(&Counts::ZERO));
    }

    #[test]
    fn goal_completes_only_when_every_epic_is_done() {
        let all_done = Counts {
            total: 3,
            done: 3,
            cancelled: 0,
            waived: 0,
        };
        assert!(goal_completed(&all_done));

        let one_cancelled = Counts {
            total: 3,
            done: 2,
            cancelled: 1,
            waived: 0,
        };
        assert!(!goal_completed(&one_cancelled));
    }
}
