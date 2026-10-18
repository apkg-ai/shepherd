use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::claim::{serde_actor_id, serde_datetime, serde_task_id, serde_uuid, serde_uuid_vec};
use super::content::serde_revision;
use super::{ActorId, ReviewPolicy, Revision, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionKind {
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "work")]
    Work,
}

impl SubmissionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SubmissionKind::Plan => "plan",
            SubmissionKind::Work => "work",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plan" => Some(SubmissionKind::Plan),
            "work" => Some(SubmissionKind::Work),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "accepted")]
    Accepted,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "withdrawn")]
    Withdrawn,
}

impl SubmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SubmissionStatus::Pending => "pending",
            SubmissionStatus::Accepted => "accepted",
            SubmissionStatus::Rejected => "rejected",
            SubmissionStatus::Withdrawn => "withdrawn",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(SubmissionStatus::Pending),
            "accepted" => Some(SubmissionStatus::Accepted),
            "rejected" => Some(SubmissionStatus::Rejected),
            "withdrawn" => Some(SubmissionStatus::Withdrawn),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    #[serde(rename = "approve")]
    Approve,
    #[serde(rename = "reject")]
    Reject,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewDecision::Approve => "approve",
            ReviewDecision::Reject => "reject",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "approve" => Some(ReviewDecision::Approve),
            "reject" => Some(ReviewDecision::Reject),
            _ => None,
        }
    }
}

// Wire Submission (plan/contracts): withdraw_reason is persisted-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    #[serde(with = "serde_revision")]
    pub revision: Revision,
    #[serde(with = "serde_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "serde_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "serde_task_id")]
    pub task_id: TaskId,
    pub kind: SubmissionKind,
    #[serde(with = "serde_actor_id")]
    pub producer_id: ActorId,
    #[serde(with = "serde_uuid_vec")]
    pub document_revision_ids: Vec<Uuid>,
    #[serde(with = "serde_uuid")]
    pub session_id: Uuid,
    pub policy: ReviewPolicy,
    pub status: SubmissionStatus,
    pub created_context_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    #[serde(with = "serde_uuid")]
    pub submission_id: Uuid,
    #[serde(with = "serde_actor_id")]
    pub actor_id: ActorId,
    pub decision: ReviewDecision,
    pub reason: String,
    #[serde(with = "serde_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub decision: ReviewDecision,
    pub reason: String,
    pub claim_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_kind_round_trips_stored_strings() {
        for kind in [SubmissionKind::Plan, SubmissionKind::Work] {
            assert_eq!(SubmissionKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SubmissionKind::parse("review"), None);
    }

    #[test]
    fn submission_status_round_trips_stored_strings() {
        for status in [
            SubmissionStatus::Pending,
            SubmissionStatus::Accepted,
            SubmissionStatus::Rejected,
            SubmissionStatus::Withdrawn,
        ] {
            assert_eq!(SubmissionStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(SubmissionStatus::parse("active"), None);
    }

    #[test]
    fn review_decision_round_trips_stored_strings() {
        for decision in [ReviewDecision::Approve, ReviewDecision::Reject] {
            assert_eq!(ReviewDecision::parse(decision.as_str()), Some(decision));
        }
        assert_eq!(ReviewDecision::parse("withdraw"), None);
    }

    #[test]
    fn review_decision_serde_uses_wire_vocabulary() {
        let json = serde_json::to_string(&ReviewDecision::Approve).unwrap();
        assert_eq!(json, r#""approve""#);
        let parsed: ReviewDecision = serde_json::from_str(r#""reject""#).unwrap();
        assert_eq!(parsed, ReviewDecision::Reject);
    }

    #[test]
    fn submission_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let submission = Submission {
            id: Uuid::nil(),
            revision: Revision::INITIAL,
            created_at: now,
            updated_at: now,
            task_id: TaskId::from_uuid(Uuid::nil()),
            kind: SubmissionKind::Plan,
            producer_id: ActorId::from_uuid(Uuid::nil()),
            document_revision_ids: vec![Uuid::nil()],
            session_id: Uuid::nil(),
            policy: ReviewPolicy::Human,
            status: SubmissionStatus::Pending,
            created_context_revision: 1,
        };
        let json = serde_json::to_string(&submission).unwrap();
        let parsed: Submission = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, submission);
    }

    #[test]
    fn review_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let review = Review {
            id: Uuid::nil(),
            submission_id: Uuid::nil(),
            actor_id: ActorId::from_uuid(Uuid::nil()),
            decision: ReviewDecision::Reject,
            reason: "missing output".into(),
            created_at: now,
        };
        let json = serde_json::to_string(&review).unwrap();
        let parsed: Review = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, review);
    }
}
