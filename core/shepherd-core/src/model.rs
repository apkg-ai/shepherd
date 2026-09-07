//! Domain types: entities, value objects, inputs, and pagination.
//!
//! Every type here mirrors the OpenAPI schemas in `openapi/shepherd.yaml`.
//! Wire-format concerns (snake_case, nullable vs Option) live on the serde
//! derives; storage concerns live in `store.rs`.

use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;

// ── Newtype IDs ──────────────────────────────────────────────────────────

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a new time-sortable UUIDv7.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wrap an existing UUID (for imports / tests).
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

define_id!(ProjectId);
define_id!(TaskId);
define_id!(RelationId);
define_id!(ClaimId);
define_id!(SessionId);
define_id!(KnowledgeId);

// ── Enums ────────────────────────────────────────────────────────────────

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$vmeta:meta])* $variant:ident => $wire:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $wire)] $variant ),+
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self {
                    $( Self::$variant => $wire ),+
                })
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $wire => Ok(Self::$variant), )+
                    other => Err(format!("invalid {}: {other}", stringify!($name))),
                }
            }
        }
    };
}

string_enum! {
    /// Lifecycle status of a task.
    TaskStatus {
        Proposed => "proposed",
        Approved => "approved",
        Ready => "ready",
        InProgress => "in_progress",
        InReview => "in_review",
        Done => "done",
        Blocked => "blocked",
        Cancelled => "cancelled",
    }
}

string_enum! {
    /// Built-in task types (conventions, not separate schemas).
    TaskType {
        Code => "code",
        Question => "question",
        Refactor => "refactor",
        Review => "review",
        Research => "research",
    }
}

string_enum! {
    /// Relation kinds: decomposition (parent/child) vs dependency.
    RelationType {
        Decomposition => "decomposition",
        DependsOn => "depends_on",
    }
}

string_enum! {
    /// Outcome of a work session.
    SessionOutcome {
        Succeeded => "succeeded",
        Failed => "failed",
    }
}

string_enum! {
    /// Knowledge item type.
    KnowledgeType {
        Link => "link",
        Transcript => "transcript",
        Decision => "decision",
        Note => "note",
    }
}

string_enum! {
    /// Scope of a knowledge item.
    KnowledgeScope {
        Task => "task",
        Session => "session",
        Project => "project",
    }
}

string_enum! {
    /// Graph positioning hint.
    GraphRole {
        Start => "start",
        End => "end",
        Milestone => "milestone",
    }
}

// ── Value objects ─────────────────────────────────────────────────────────

/// Self-declared caller descriptor. Not authentication — callers identify
/// themselves on claim and report calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub harness: String,
    pub agent_model: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Identity {
    /// Compare on the three required fields only (label is optional context).
    pub fn matches(&self, other: &Identity) -> bool {
        self.harness == other.harness
            && self.agent_model == other.agent_model
            && self.session_id == other.session_id
    }
}

// ── Entity structs ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub settings: ProjectSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub review_gate: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { review_gate: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    #[serde(rename = "type")]
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub metadata: serde_json::Value,
    pub assignee: Option<Identity>,
    pub graph_role: Vec<GraphRole>,
    pub attempt_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_from_status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: RelationId,
    #[serde(rename = "type")]
    pub relation_type: RelationType,
    pub source_task_id: TaskId,
    pub target_task_id: TaskId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: ClaimId,
    pub task_id: TaskId,
    pub identity: Identity,
    pub ttl_seconds: i32,
    pub lease_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub renewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub task_id: TaskId,
    pub identity: Identity,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub outcome: SessionOutcome,
    pub failure_reason: Option<String>,
    pub summary: String,
    pub decisions: Vec<String>,
    pub knowledge_items: Vec<KnowledgeItem>,
    pub artifacts: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeItem {
    pub id: KnowledgeId,
    #[serde(rename = "type")]
    pub knowledge_type: KnowledgeType,
    pub title: String,
    pub content: String,
    pub scope: KnowledgeScope,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub project_id: ProjectId,
    pub created_at: DateTime<Utc>,
}

// ── Input / request structs ──────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectCreate {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub settings: Option<ProjectSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<ProjectSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskCreate {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub task_type: TaskType,
    /// Only `proposed` (default) or `approved` allowed at creation.
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub assignee: Option<Identity>,
    #[serde(default)]
    pub graph_role: Option<Vec<GraphRole>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub task_type: Option<TaskType>,
    pub metadata: Option<serde_json::Value>,
    pub assignee: Option<Option<Identity>>,
    pub graph_role: Option<Vec<GraphRole>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelationCreate {
    #[serde(rename = "type")]
    pub relation_type: RelationType,
    pub target_task_id: TaskId,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimRequest {
    pub identity: Identity,
    pub ttl_seconds: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimRenewal {
    pub identity: Identity,
    pub ttl_seconds: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimRelease {
    pub identity: Identity,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionReport {
    pub identity: Identity,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub outcome: SessionOutcome,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub decisions: Option<Vec<String>>,
    #[serde(default)]
    pub knowledge_items: Option<Vec<KnowledgeItemCreate>>,
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KnowledgeItemCreate {
    #[serde(rename = "type")]
    pub knowledge_type: KnowledgeType,
    pub title: String,
    pub content: String,
    #[serde(default = "default_project_scope")]
    pub scope: KnowledgeScope,
    #[serde(default)]
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
}

fn default_project_scope() -> KnowledgeScope {
    KnowledgeScope::Project
}

// ── Computed / response structs ──────────────────────────────────────────

/// Context bundle assembled on task claim.
#[derive(Debug, Clone, Serialize)]
pub struct ContextBundle {
    pub task: Task,
    pub ancestor_summaries: Vec<AncestorSummary>,
    pub project_knowledge: Vec<KnowledgeItem>,
    pub artifacts: Vec<String>,
    pub sibling_tasks: Vec<SiblingTask>,
}

/// Summary of an ancestor task (dependency or parent chain).
#[derive(Debug, Clone, Serialize)]
pub struct AncestorSummary {
    pub task_id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub summary: String,
    pub decisions: Vec<String>,
}

/// An in-flight sibling task (prevents duplicated work).
#[derive(Debug, Clone, Serialize)]
pub struct SiblingTask {
    pub task_id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub claimed_by: Option<Identity>,
}

// ── Export / Import ──────────────────────────────────────────────────────

/// Versioned project export document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDocument {
    pub version: String,
    pub exported_at: DateTime<Utc>,
    pub project: Project,
    pub tasks: Vec<Task>,
    pub relations: Vec<Relation>,
    pub sessions: Vec<Session>,
    pub knowledge_items: Vec<KnowledgeItem>,
}

/// Result of importing a project.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub project_id: ProjectId,
    pub task_count: usize,
    pub relation_count: usize,
    pub session_count: usize,
    pub knowledge_count: usize,
}

/// Result of purging soft-deleted records.
#[derive(Debug, Clone, Serialize)]
pub struct PurgeResult {
    pub projects_purged: usize,
    pub tasks_purged: usize,
}

// ── Pagination ───────────────────────────────────────────────────────────

/// Opaque cursor for keyset pagination, encoding `(created_at, id)`.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

impl Cursor {
    /// Encode the cursor as a URL-safe base64 string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(&(self.created_at.to_rfc3339(), self.id))
            .expect("cursor serialization is infallible");
        URL_SAFE_NO_PAD.encode(json.as_bytes())
    }

    /// Decode a cursor from a URL-safe base64 string.
    pub fn decode(s: &str) -> Result<Self, Error> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| Error::ValidationError {
                detail: "invalid cursor".into(),
                errors: vec![],
            })?;
        let json = String::from_utf8(bytes).map_err(|_| Error::ValidationError {
            detail: "invalid cursor encoding".into(),
            errors: vec![],
        })?;
        let (ts_str, id): (String, Uuid) =
            serde_json::from_str(&json).map_err(|_| Error::ValidationError {
                detail: "invalid cursor format".into(),
                errors: vec![],
            })?;
        let created_at = DateTime::parse_from_rfc3339(&ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| Error::ValidationError {
                detail: "invalid cursor timestamp".into(),
                errors: vec![],
            })?;
        Ok(Self { created_at, id })
    }
}

/// Generic paginated response.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T: Serialize> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

impl<T: Serialize> Page<T> {
    /// Build a page from a query result that fetched `limit + 1` rows.
    pub fn from_rows(mut items: Vec<T>, limit: usize, cursor_fn: impl Fn(&T) -> Cursor) -> Self {
        let has_more = items.len() > limit;
        if has_more {
            items.truncate(limit);
        }
        let next_cursor = if has_more {
            items.last().map(|t| cursor_fn(t).encode())
        } else {
            None
        };
        Self {
            items,
            has_more,
            next_cursor,
        }
    }
}

// ── Validation helpers ───────────────────────────────────────────────────

/// Maximum number of top-level properties in task metadata JSON.
pub const MAX_METADATA_PROPERTIES: usize = 200;

/// Validate task metadata JSON constraints.
pub fn validate_metadata(metadata: &serde_json::Value) -> Result<(), Error> {
    match metadata {
        serde_json::Value::Object(map) if map.len() > MAX_METADATA_PROPERTIES => {
            Err(Error::ValidationError {
                detail: format!("metadata exceeds {MAX_METADATA_PROPERTIES} properties"),
                errors: vec![],
            })
        }
        serde_json::Value::Object(_) => Ok(()),
        _ => Err(Error::ValidationError {
            detail: "metadata must be a JSON object".into(),
            errors: vec![],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_roundtrip() {
        for s in [
            "proposed",
            "approved",
            "ready",
            "in_progress",
            "in_review",
            "done",
            "blocked",
            "cancelled",
        ] {
            let status: TaskStatus = s.parse().unwrap();
            assert_eq!(status.to_string(), s);
        }
    }

    #[test]
    fn task_type_roundtrip() {
        for s in ["code", "question", "refactor", "review", "research"] {
            let tt: TaskType = s.parse().unwrap();
            assert_eq!(tt.to_string(), s);
        }
    }

    #[test]
    fn identity_matches_ignores_label() {
        let a = Identity {
            harness: "cc".into(),
            agent_model: "opus".into(),
            session_id: "s1".into(),
            label: Some("label-a".into()),
        };
        let b = Identity {
            harness: "cc".into(),
            agent_model: "opus".into(),
            session_id: "s1".into(),
            label: Some("label-b".into()),
        };
        assert!(a.matches(&b));
    }

    #[test]
    fn identity_mismatch_on_required_field() {
        let a = Identity {
            harness: "cc".into(),
            agent_model: "opus".into(),
            session_id: "s1".into(),
            label: None,
        };
        let b = Identity {
            harness: "cursor".into(),
            agent_model: "opus".into(),
            session_id: "s1".into(),
            label: None,
        };
        assert!(!a.matches(&b));
    }

    #[test]
    fn cursor_roundtrip() {
        let now = Utc::now();
        let id = Uuid::now_v7();
        let cursor = Cursor {
            created_at: now,
            id,
        };
        let encoded = cursor.encode();
        let decoded = Cursor::decode(&encoded).unwrap();
        assert_eq!(decoded.id, id);
        // chrono round-trip may lose sub-nanosecond precision but stays within
        // a second; we only need millisecond-level accuracy for pagination.
        assert!(
            (decoded.created_at - now).num_milliseconds().abs() < 1000,
            "cursor timestamp drift too large"
        );
    }

    #[test]
    fn cursor_decode_invalid() {
        assert!(Cursor::decode("not-valid-base64!!!").is_err());
    }

    #[test]
    fn validate_metadata_ok() {
        let val = serde_json::json!({"key": "value"});
        assert!(validate_metadata(&val).is_ok());
    }

    #[test]
    fn validate_metadata_too_many_properties() {
        let mut map = serde_json::Map::new();
        for i in 0..=MAX_METADATA_PROPERTIES {
            map.insert(format!("k{i}"), serde_json::Value::Null);
        }
        let val = serde_json::Value::Object(map);
        assert!(validate_metadata(&val).is_err());
    }

    #[test]
    fn validate_metadata_must_be_object() {
        let val = serde_json::json!([1, 2, 3]);
        assert!(validate_metadata(&val).is_err());
    }

    #[test]
    fn page_from_rows_no_more() {
        let items = vec![1, 2, 3];
        let page = Page::from_rows(items, 5, |_| Cursor {
            created_at: Utc::now(),
            id: Uuid::now_v7(),
        });
        assert_eq!(page.items.len(), 3);
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn page_from_rows_has_more() {
        let items = vec![1, 2, 3, 4, 5, 6]; // limit=5, got 6
        let page = Page::from_rows(items, 5, |_| Cursor {
            created_at: Utc::now(),
            id: Uuid::now_v7(),
        });
        assert_eq!(page.items.len(), 5);
        assert!(page.has_more);
        assert!(page.next_cursor.is_some());
    }
}
