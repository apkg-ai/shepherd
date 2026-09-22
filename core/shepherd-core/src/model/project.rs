use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Counts, LifecycleRecord, Revision, typed_uuid, validate_required_text};
use crate::error::DomainError;

typed_uuid!(ProjectId);
typed_uuid!(TaskTypeId);

pub(crate) const NAME_MAX_CHARS: usize = 200;
pub(crate) const DESCRIPTION_MAX_CHARS: usize = 10_000;
pub(crate) const LABEL_MAX_CHARS: usize = 100;
const KEY_MAX_LEN: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPolicy {
    Human,
    Agent,
    None,
}

impl ReviewPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewPolicy::Human => "human",
            ReviewPolicy::Agent => "agent",
            ReviewPolicy::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    pub proposal_gate: bool,
    pub planning_required: bool,
    pub plan_review: ReviewPolicy,
    pub work_review: ReviewPolicy,
}

impl Default for ProjectSettings {
    // Defaults fixed by the product scope (plan/00).
    fn default() -> Self {
        Self {
            proposal_gate: true,
            planning_required: false,
            plan_review: ReviewPolicy::Human,
            work_review: ReviewPolicy::Human,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    pub revision: Revision,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub name: String,
    pub description: String,
    pub settings: ProjectSettings,
    pub archived: bool,
    pub epic_counts: Counts,
    pub archive: Option<LifecycleRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskType {
    pub id: TaskTypeId,
    pub revision: Revision,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_id: ProjectId,
    pub key: String,
    pub label: String,
    pub archived: bool,
    pub builtin: bool,
}

pub const BUILTIN_TASK_TYPES: [(&str, &str); 6] = [
    ("code", "Code"),
    ("research", "Research"),
    ("design", "Design"),
    ("documentation", "Documentation"),
    ("test", "Test"),
    ("other", "Other"),
];

#[derive(Debug, Clone, Default)]
pub struct ProjectCreate {
    pub name: String,
    pub description: Option<String>,
    pub settings: Option<ProjectSettings>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Full snapshot replacement; the contract has no per-field settings patch.
    pub settings: Option<ProjectSettings>,
}

#[derive(Debug, Clone)]
pub struct TaskTypeCreate {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct TaskTypePatch {
    pub label: Option<String>,
}

// Contract pattern ^[a-z][a-z0-9_]{0,39}$; keys are immutable once created.
pub(crate) fn validate_type_key(value: &str) -> Result<String, DomainError> {
    let bytes = value.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= KEY_MAX_LEN
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_');
    if valid {
        Ok(value.to_string())
    } else {
        Err(DomainError::Validation {
            field: "key",
            message: "must match ^[a-z][a-z0-9_]{0,39}$".into(),
        })
    }
}

pub(crate) fn validate_type_label(value: &str) -> Result<String, DomainError> {
    validate_required_text("label", value, LABEL_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_the_documented_snapshot() {
        let settings = ProjectSettings::default();
        assert!(settings.proposal_gate);
        assert!(!settings.planning_required);
        assert_eq!(settings.plan_review, ReviewPolicy::Human);
        assert_eq!(settings.work_review, ReviewPolicy::Human);
    }

    #[test]
    fn settings_round_trip_through_snake_case_json() {
        let json = serde_json::to_string(&ProjectSettings::default()).unwrap();
        assert_eq!(
            json,
            "{\"proposal_gate\":true,\"planning_required\":false,\
             \"plan_review\":\"human\",\"work_review\":\"human\"}"
        );
        let parsed: ProjectSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ProjectSettings::default());
    }

    #[test]
    fn settings_reject_unknown_fields() {
        let err = serde_json::from_str::<ProjectSettings>(
            "{\"proposal_gate\":true,\"planning_required\":false,\
             \"plan_review\":\"human\",\"work_review\":\"human\",\"extra\":1}",
        )
        .unwrap_err();
        assert!(err.to_string().contains("extra"));
    }

    #[test]
    fn type_keys_follow_the_contract_pattern() {
        assert_eq!(validate_type_key("code").unwrap(), "code");
        assert_eq!(validate_type_key("a_1").unwrap(), "a_1");
        assert_eq!(validate_type_key(&"a".repeat(40)).unwrap(), "a".repeat(40));
        for bad in ["", "Bad", "1abc", "_abc", "has-dash", "has space"] {
            assert!(matches!(
                validate_type_key(bad),
                Err(DomainError::Validation { field: "key", .. })
            ));
        }
        assert!(validate_type_key(&"a".repeat(41)).is_err());
    }

    #[test]
    fn type_labels_are_trimmed_and_bounded() {
        assert_eq!(validate_type_label(" Code ").unwrap(), "Code");
        assert!(validate_type_label("   ").is_err());
        assert!(validate_type_label(&"x".repeat(101)).is_err());
    }

    #[test]
    fn builtin_registry_covers_the_six_documented_types() {
        let keys: Vec<&str> = BUILTIN_TASK_TYPES.iter().map(|(key, _)| *key).collect();
        assert_eq!(
            keys,
            [
                "code",
                "research",
                "design",
                "documentation",
                "test",
                "other"
            ]
        );
        for (key, label) in BUILTIN_TASK_TYPES {
            assert_eq!(validate_type_key(key).unwrap(), key);
            assert_eq!(label.to_lowercase(), key);
        }
    }
}
