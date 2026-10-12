use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::claim::{serde_actor_id, serde_datetime, serde_task_id, serde_uuid, serde_uuid_vec};
use super::{ActorId, ClaimPhase, TaskId};
use crate::error::DomainError;

pub(crate) const SUMMARY_MAX_CHARS: usize = 10_000;
pub(crate) const FAILURE_REASON_MAX_CHARS: usize = 2_000;
// Wire limits from plan/contracts ReportInput: maxItems on both arrays and
// maxLength on each link entry.
pub(crate) const MAX_DOCUMENT_REVISION_IDS: usize = 50;
pub(crate) const MAX_LINKS: usize = 50;
pub(crate) const LINK_MAX_CHARS: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionOutcome {
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "failed")]
    Failed,
}

impl SessionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionOutcome::Succeeded => "succeeded",
            SessionOutcome::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "succeeded" => Some(SessionOutcome::Succeeded),
            "failed" => Some(SessionOutcome::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    #[serde(with = "serde_task_id")]
    pub task_id: TaskId,
    #[serde(with = "serde_uuid")]
    pub claim_id: Uuid,
    #[serde(with = "serde_actor_id")]
    pub actor_id: ActorId,
    pub phase: ClaimPhase,
    #[serde(with = "serde_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(with = "serde_datetime")]
    pub ended_at: DateTime<Utc>,
    pub outcome: SessionOutcome,
    pub summary: String,
    pub failure_reason: String,
    #[serde(with = "serde_uuid_vec")]
    pub document_revision_ids: Vec<Uuid>,
    pub links: serde_json::Value,
}

pub struct ReportInput {
    pub outcome: SessionOutcome,
    pub summary: String,
    pub failure_reason: String,
    pub document_revision_ids: Vec<Uuid>,
    pub links: serde_json::Value,
    // Step-010 seam: step 011's selectTaskPlan persists the selection on the
    // task; reportClaim then reads it instead of receiving it here.
    /// Required when outcome=Succeeded and phase=Plan.
    pub selected_plan_revision_id: Option<Uuid>,
}

pub(crate) fn validate_report_input(
    input: &ReportInput,
    phase: ClaimPhase,
) -> Result<(), DomainError> {
    super::validate_long_text("summary", &input.summary, SUMMARY_MAX_CHARS)?;
    if input.summary.trim().is_empty() {
        return Err(DomainError::Validation {
            field: "summary",
            message: "must not be blank".into(),
        });
    }
    match input.outcome {
        SessionOutcome::Failed => {
            let trimmed = input.failure_reason.trim();
            if trimmed.is_empty() {
                return Err(DomainError::Validation {
                    field: "failure_reason",
                    message: "must not be blank for failed reports".into(),
                });
            }
            super::validate_long_text(
                "failure_reason",
                &input.failure_reason,
                FAILURE_REASON_MAX_CHARS,
            )?;
        }
        SessionOutcome::Succeeded => {
            if !input.failure_reason.is_empty() {
                return Err(DomainError::Validation {
                    field: "failure_reason",
                    message: "must be empty for successful reports".into(),
                });
            }
            if phase == ClaimPhase::Plan && input.selected_plan_revision_id.is_none() {
                return Err(DomainError::Validation {
                    field: "selected_plan_revision_id",
                    message: "required for successful plan reports".into(),
                });
            }
        }
    }
    if input.selected_plan_revision_id.is_some()
        && !(input.outcome == SessionOutcome::Succeeded && phase == ClaimPhase::Plan)
    {
        return Err(DomainError::Validation {
            field: "selected_plan_revision_id",
            message: "only valid for successful plan reports".into(),
        });
    }
    if input.document_revision_ids.len() > MAX_DOCUMENT_REVISION_IDS {
        return Err(DomainError::Validation {
            field: "document_revision_ids",
            message: format!("must contain at most {MAX_DOCUMENT_REVISION_IDS} entries"),
        });
    }
    match input.links.as_array() {
        Some(links) => {
            if links.len() > MAX_LINKS {
                return Err(DomainError::Validation {
                    field: "links",
                    message: format!("must contain at most {MAX_LINKS} entries"),
                });
            }
            for link in links {
                if !link
                    .as_str()
                    .is_some_and(|value| value.chars().count() <= LINK_MAX_CHARS)
                {
                    return Err(DomainError::Validation {
                        field: "links",
                        message: format!(
                            "each entry must be a string of at most {LINK_MAX_CHARS} characters"
                        ),
                    });
                }
            }
        }
        None => {
            return Err(DomainError::Validation {
                field: "links",
                message: "must be a JSON array".into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_outcome_round_trips_stored_strings() {
        for outcome in [SessionOutcome::Succeeded, SessionOutcome::Failed] {
            assert_eq!(SessionOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(SessionOutcome::parse("partial"), None);
    }

    #[test]
    fn session_outcome_serde_uses_wire_vocabulary() {
        let json = serde_json::to_string(&SessionOutcome::Succeeded).unwrap();
        assert_eq!(json, r#""succeeded""#);
        let parsed: SessionOutcome = serde_json::from_str(r#""failed""#).unwrap();
        assert_eq!(parsed, SessionOutcome::Failed);
    }

    #[test]
    fn session_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let session = Session {
            id: Uuid::nil(),
            task_id: TaskId::from_uuid(Uuid::nil()),
            claim_id: Uuid::nil(),
            actor_id: ActorId::from_uuid(Uuid::nil()),
            phase: ClaimPhase::Execute,
            started_at: now,
            ended_at: now,
            outcome: SessionOutcome::Succeeded,
            summary: "done".into(),
            failure_reason: String::new(),
            document_revision_ids: vec![Uuid::nil()],
            links: serde_json::json!([]),
        };
        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, session);
    }

    #[test]
    fn validate_rejects_blank_failure_reason_on_failed_report() {
        let input = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "s".into(),
            failure_reason: String::new(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "failure_reason",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_nonempty_failure_reason_on_succeeded_report() {
        let input = ReportInput {
            outcome: SessionOutcome::Succeeded,
            summary: "s".into(),
            failure_reason: "oops".into(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "failure_reason",
                ..
            }
        ));
    }

    #[test]
    fn validate_requires_selected_plan_revision_for_successful_plan() {
        let input = ReportInput {
            outcome: SessionOutcome::Succeeded,
            summary: "s".into(),
            failure_reason: String::new(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Plan).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "selected_plan_revision_id",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_non_array_links() {
        let input = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "s".into(),
            failure_reason: "boom".into(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!({}),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "links", .. }
        ));
    }

    #[test]
    fn validate_rejects_blank_summary() {
        let input = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: String::new(),
            failure_reason: "boom".into(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "summary",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_whitespace_only_summary() {
        let input = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "   ".into(),
            failure_reason: "boom".into(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "summary",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_oversize_summary_and_failure_reason() {
        let oversize_summary = ReportInput {
            outcome: SessionOutcome::Succeeded,
            summary: "x".repeat(SUMMARY_MAX_CHARS + 1),
            failure_reason: String::new(),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&oversize_summary, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "summary",
                ..
            }
        ));
        let oversize_reason = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "s".into(),
            failure_reason: "x".repeat(FAILURE_REASON_MAX_CHARS + 1),
            document_revision_ids: Vec::new(),
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&oversize_reason, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "failure_reason",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_more_than_50_revisions() {
        let input = ReportInput {
            outcome: SessionOutcome::Failed,
            summary: "s".into(),
            failure_reason: "boom".into(),
            document_revision_ids: vec![Uuid::nil(); MAX_DOCUMENT_REVISION_IDS + 1],
            links: serde_json::json!([]),
            selected_plan_revision_id: None,
        };
        let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation {
                field: "document_revision_ids",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_oversized_and_non_string_links() {
        for links in [
            serde_json::json!(["ok", 7]),
            serde_json::json!([{"uri": "x"}]),
            serde_json::json!(["x".repeat(LINK_MAX_CHARS + 1)]),
            serde_json::json!((0..MAX_LINKS + 1).map(|_| "x").collect::<Vec<_>>()),
        ] {
            let input = ReportInput {
                outcome: SessionOutcome::Failed,
                summary: "s".into(),
                failure_reason: "boom".into(),
                document_revision_ids: Vec::new(),
                links: links.clone(),
                selected_plan_revision_id: None,
            };
            let err = validate_report_input(&input, ClaimPhase::Execute).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation { field: "links", .. }),
                "links {links} must be rejected"
            );
        }
    }

    #[test]
    fn validate_rejects_selected_plan_revision_outside_plan_success() {
        for (outcome, phase) in [
            (SessionOutcome::Failed, ClaimPhase::Plan),
            (SessionOutcome::Succeeded, ClaimPhase::Execute),
        ] {
            let failure_reason = if outcome == SessionOutcome::Failed {
                "boom".to_string()
            } else {
                String::new()
            };
            let input = ReportInput {
                outcome,
                summary: "s".into(),
                failure_reason,
                document_revision_ids: Vec::new(),
                links: serde_json::json!([]),
                selected_plan_revision_id: Some(Uuid::nil()),
            };
            let err = validate_report_input(&input, phase).unwrap_err();
            assert!(
                matches!(
                    err,
                    DomainError::Validation {
                        field: "selected_plan_revision_id",
                        ..
                    }
                ),
                "outcome {outcome:?} phase {phase:?} must be rejected"
            );
        }
    }
}
