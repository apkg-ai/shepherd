use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::claim::{serde_actor_id, serde_datetime, serde_uuid};
use super::{ActorId, EpicId, ProjectId, Revision, TaskId, typed_uuid};
use crate::error::DomainError;

pub(crate) const DOC_TITLE_MAX_CHARS: usize = 200;
pub(crate) const DOC_BODY_MAX_CHARS: usize = 65_536;
// Wire limits shared with ReportInput links (plan/contracts).
pub(crate) const MAX_LINKS: usize = 50;
pub(crate) const LINK_MAX_CHARS: usize = 2_048;

typed_uuid!(DocumentId);
typed_uuid!(DocumentRevisionId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentOwnerKind {
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "epic")]
    Epic,
    #[serde(rename = "task")]
    Task,
}

impl DocumentOwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentOwnerKind::Project => "project",
            DocumentOwnerKind::Epic => "epic",
            DocumentOwnerKind::Task => "task",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(DocumentOwnerKind::Project),
            "epic" => Some(DocumentOwnerKind::Epic),
            "task" => Some(DocumentOwnerKind::Task),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentKind {
    #[serde(rename = "brief")]
    Brief,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "finding")]
    Finding,
    #[serde(rename = "handoff")]
    Handoff,
    #[serde(rename = "note")]
    Note,
    #[serde(rename = "decision")]
    Decision,
}

impl DocumentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Brief => "brief",
            DocumentKind::Plan => "plan",
            DocumentKind::Finding => "finding",
            DocumentKind::Handoff => "handoff",
            DocumentKind::Note => "note",
            DocumentKind::Decision => "decision",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "brief" => Some(DocumentKind::Brief),
            "plan" => Some(DocumentKind::Plan),
            "finding" => Some(DocumentKind::Finding),
            "handoff" => Some(DocumentKind::Handoff),
            "note" => Some(DocumentKind::Note),
            "decision" => Some(DocumentKind::Decision),
            _ => None,
        }
    }

    // Owner decision 2026-10-12: terminal unarchived entities accept only
    // contextual notes (plan/03 "plans ... cannot be replaced").
    pub(crate) fn allowed_on_terminal_owner(self) -> bool {
        matches!(self, DocumentKind::Note)
    }
}

mod serde_document_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    use super::DocumentId;

    pub fn serialize<S: Serializer>(id: &DocumentId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DocumentId, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text)
            .map(DocumentId::from_uuid)
            .map_err(serde::de::Error::custom)
    }
}

mod serde_document_revision_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    use super::DocumentRevisionId;

    pub fn serialize<S: Serializer>(id: &DocumentRevisionId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DocumentRevisionId, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text)
            .map(DocumentRevisionId::from_uuid)
            .map_err(serde::de::Error::custom)
    }
}

mod serde_project_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    use crate::model::ProjectId;

    pub fn serialize<S: Serializer>(id: &ProjectId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ProjectId, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text)
            .map(ProjectId::from_uuid)
            .map_err(serde::de::Error::custom)
    }
}

mod serde_revision {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::model::Revision;

    pub fn serialize<S: Serializer>(revision: &Revision, s: S) -> Result<S::Ok, S::Error> {
        revision.value().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Revision, D::Error> {
        let value = i64::deserialize(d)?;
        Revision::from_stored(value)
            .ok_or_else(|| serde::de::Error::custom("revision must be at least 1"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    #[serde(with = "serde_document_id")]
    pub id: DocumentId,
    #[serde(with = "serde_revision")]
    pub revision: Revision,
    #[serde(with = "serde_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "serde_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "serde_project_id")]
    pub project_id: ProjectId,
    pub owner_kind: DocumentOwnerKind,
    #[serde(with = "serde_uuid")]
    pub owner_id: Uuid,
    pub kind: DocumentKind,
    pub title: String,
    #[serde(with = "serde_document_revision_id")]
    pub latest_revision_id: DocumentRevisionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRevision {
    #[serde(with = "serde_document_revision_id")]
    pub id: DocumentRevisionId,
    #[serde(with = "serde_document_id")]
    pub document_id: DocumentId,
    pub number: i64,
    pub body: String,
    pub links: Vec<String>,
    #[serde(with = "serde_actor_id")]
    pub actor_id: ActorId,
    #[serde(with = "serde_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOwner {
    Project(ProjectId),
    Epic(EpicId),
    Task(TaskId),
}

impl DocumentOwner {
    pub fn from_parts(kind: DocumentOwnerKind, id: Uuid) -> Self {
        match kind {
            DocumentOwnerKind::Project => DocumentOwner::Project(ProjectId::from_uuid(id)),
            DocumentOwnerKind::Epic => DocumentOwner::Epic(EpicId::from_uuid(id)),
            DocumentOwnerKind::Task => DocumentOwner::Task(TaskId::from_uuid(id)),
        }
    }

    pub fn kind(&self) -> DocumentOwnerKind {
        match self {
            DocumentOwner::Project(_) => DocumentOwnerKind::Project,
            DocumentOwner::Epic(_) => DocumentOwnerKind::Epic,
            DocumentOwner::Task(_) => DocumentOwnerKind::Task,
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            DocumentOwner::Project(id) => id.as_uuid(),
            DocumentOwner::Epic(id) => id.as_uuid(),
            DocumentOwner::Task(id) => id.as_uuid(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentCreate {
    pub owner: DocumentOwner,
    pub kind: DocumentKind,
    pub title: String,
    pub body: String,
    pub links: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RevisionCreate {
    pub body: String,
    pub links: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct SelectPlanInput {
    pub document_revision_id: Uuid,
}

/// Returns the trimmed title.
pub(crate) fn validate_document_create(input: &DocumentCreate) -> Result<String, DomainError> {
    let title = super::validate_required_text("title", &input.title, DOC_TITLE_MAX_CHARS)?;
    super::validate_long_text("body", &input.body, DOC_BODY_MAX_CHARS)?;
    validate_links("links", &input.links)?;
    Ok(title)
}

pub(crate) fn validate_revision_create(input: &RevisionCreate) -> Result<(), DomainError> {
    super::validate_long_text("body", &input.body, DOC_BODY_MAX_CHARS)?;
    validate_links("links", &input.links)
}

pub(crate) fn validate_links(field: &'static str, links: &[String]) -> Result<(), DomainError> {
    if links.len() > MAX_LINKS {
        return Err(DomainError::Validation {
            field,
            message: format!("must contain at most {MAX_LINKS} entries"),
        });
    }
    for link in links {
        validate_link_entry(field, link)?;
    }
    Ok(())
}

pub(crate) fn validate_link_entry(field: &'static str, value: &str) -> Result<(), DomainError> {
    if value.chars().count() > LINK_MAX_CHARS {
        return Err(DomainError::Validation {
            field,
            message: format!("each entry must be at most {LINK_MAX_CHARS} characters"),
        });
    }
    if !link_is_safe(value) {
        return Err(DomainError::Validation {
            field,
            message: "each entry must be an http(s) URL without embedded credentials".into(),
        });
    }
    Ok(())
}

// plan/07: HTTP(S) artifact links only; reject credentials in URLs and
// javascript/data/file schemes. Hand-rolled: no url crate is pinned for core.
fn link_is_safe(value: &str) -> bool {
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_input() -> DocumentCreate {
        DocumentCreate {
            owner: DocumentOwner::Task(TaskId::from_uuid(Uuid::nil())),
            kind: DocumentKind::Plan,
            title: "Plan".into(),
            body: "# body".into(),
            links: Vec::new(),
        }
    }

    #[test]
    fn document_owner_kind_round_trips_stored_strings() {
        for kind in [
            DocumentOwnerKind::Project,
            DocumentOwnerKind::Epic,
            DocumentOwnerKind::Task,
        ] {
            assert_eq!(DocumentOwnerKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DocumentOwnerKind::parse("goal"), None);
    }

    #[test]
    fn document_kind_round_trips_stored_strings() {
        for kind in [
            DocumentKind::Brief,
            DocumentKind::Plan,
            DocumentKind::Finding,
            DocumentKind::Handoff,
            DocumentKind::Note,
            DocumentKind::Decision,
        ] {
            assert_eq!(DocumentKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DocumentKind::parse("report"), None);
    }

    #[test]
    fn only_notes_are_allowed_on_terminal_owners() {
        assert!(DocumentKind::Note.allowed_on_terminal_owner());
        for kind in [
            DocumentKind::Brief,
            DocumentKind::Plan,
            DocumentKind::Finding,
            DocumentKind::Handoff,
            DocumentKind::Decision,
        ] {
            assert!(!kind.allowed_on_terminal_owner(), "{kind:?}");
        }
    }

    #[test]
    fn document_kind_serde_uses_wire_vocabulary() {
        let json = serde_json::to_string(&DocumentKind::Handoff).unwrap();
        assert_eq!(json, r#""handoff""#);
        let parsed: DocumentOwnerKind = serde_json::from_str(r#""epic""#).unwrap();
        assert_eq!(parsed, DocumentOwnerKind::Epic);
    }

    #[test]
    fn document_owner_reports_kind_and_id() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let project = ProjectId::generate(now);
        let epic = EpicId::generate(now);
        let task = TaskId::generate(now);
        let cases = [
            (
                DocumentOwner::Project(project),
                DocumentOwnerKind::Project,
                project.as_uuid(),
            ),
            (
                DocumentOwner::Epic(epic),
                DocumentOwnerKind::Epic,
                epic.as_uuid(),
            ),
            (
                DocumentOwner::Task(task),
                DocumentOwnerKind::Task,
                task.as_uuid(),
            ),
        ];
        for (owner, kind, id) in cases {
            assert_eq!(owner.kind(), kind);
            assert_eq!(owner.id(), id);
            assert_eq!(DocumentOwner::from_parts(kind, id), owner);
        }
    }

    #[test]
    fn document_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let document = Document {
            id: DocumentId::from_uuid(Uuid::nil()),
            revision: Revision::INITIAL,
            created_at: now,
            updated_at: now,
            project_id: ProjectId::from_uuid(Uuid::nil()),
            owner_kind: DocumentOwnerKind::Task,
            owner_id: Uuid::nil(),
            kind: DocumentKind::Plan,
            title: "Plan".into(),
            latest_revision_id: DocumentRevisionId::from_uuid(Uuid::nil()),
        };
        let json = serde_json::to_string(&document).unwrap();
        let parsed: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, document);
    }

    #[test]
    fn document_revision_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let revision = DocumentRevision {
            id: DocumentRevisionId::from_uuid(Uuid::nil()),
            document_id: DocumentId::from_uuid(Uuid::nil()),
            number: 2,
            body: "# body".into(),
            links: vec!["https://example.com/artifact".into()],
            actor_id: ActorId::from_uuid(Uuid::nil()),
            created_at: now,
        };
        let json = serde_json::to_string(&revision).unwrap();
        let parsed: DocumentRevision = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, revision);
    }

    #[test]
    fn stored_revision_below_one_fails_deserialization() {
        let err = serde_json::from_str::<Document>(
            r#"{"id":"00000000-0000-0000-0000-000000000000","revision":0,
               "created_at":"2026-09-14T00:00:00.000Z","updated_at":"2026-09-14T00:00:00.000Z",
               "project_id":"00000000-0000-0000-0000-000000000000","owner_kind":"task",
               "owner_id":"00000000-0000-0000-0000-000000000000","kind":"plan","title":"t",
               "latest_revision_id":"00000000-0000-0000-0000-000000000000"}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("revision must be at least 1"));
    }

    #[test]
    fn validate_document_create_trims_title() {
        let input = DocumentCreate {
            title: "  Plan  ".into(),
            ..create_input()
        };
        assert_eq!(validate_document_create(&input).unwrap(), "Plan");
    }

    #[test]
    fn validate_document_create_enforces_title_limits() {
        for title in [
            String::new(),
            "   ".into(),
            "x".repeat(DOC_TITLE_MAX_CHARS + 1),
        ] {
            let input = DocumentCreate {
                title,
                ..create_input()
            };
            let err = validate_document_create(&input).unwrap_err();
            assert!(matches!(
                err,
                DomainError::Validation { field: "title", .. }
            ));
        }
        let at_limit = DocumentCreate {
            title: "x".repeat(DOC_TITLE_MAX_CHARS),
            ..create_input()
        };
        assert!(validate_document_create(&at_limit).is_ok());
    }

    #[test]
    fn validate_document_create_enforces_body_limit() {
        let at_limit = DocumentCreate {
            body: "x".repeat(DOC_BODY_MAX_CHARS),
            ..create_input()
        };
        assert!(validate_document_create(&at_limit).is_ok());
        let oversize = DocumentCreate {
            body: "x".repeat(DOC_BODY_MAX_CHARS + 1),
            ..create_input()
        };
        let err = validate_document_create(&oversize).unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "body", .. }));
    }

    #[test]
    fn validate_revision_create_enforces_body_and_links() {
        assert!(
            validate_revision_create(&RevisionCreate {
                body: String::new(),
                links: vec!["https://example.com".into()],
            })
            .is_ok(),
            "empty body is legitimate content"
        );
        let err = validate_revision_create(&RevisionCreate {
            body: "x".repeat(DOC_BODY_MAX_CHARS + 1),
            links: Vec::new(),
        })
        .unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "body", .. }));
        let err = validate_revision_create(&RevisionCreate {
            body: String::new(),
            links: vec!["ftp://example.com".into()],
        })
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "links", .. }
        ));
    }

    #[test]
    fn validate_links_enforces_count_and_entry_length() {
        let too_many: Vec<String> = (0..MAX_LINKS + 1)
            .map(|_| "https://example.com".to_string())
            .collect();
        let err = validate_links("links", &too_many).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "links", .. }
        ));

        let at_count: Vec<String> = (0..MAX_LINKS)
            .map(|_| "https://example.com".to_string())
            .collect();
        assert!(validate_links("links", &at_count).is_ok());

        let at_length = format!(
            "https://example.com/{}",
            "x".repeat(LINK_MAX_CHARS - "https://example.com/".len())
        );
        assert_eq!(at_length.chars().count(), LINK_MAX_CHARS);
        assert!(validate_link_entry("links", &at_length).is_ok());

        let over_length = format!("{at_length}x");
        let err = validate_link_entry("links", &over_length).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "links", .. }
        ));
    }

    #[test]
    fn link_safety_accepts_http_and_https_only() {
        for link in [
            "http://example.com",
            "https://example.com",
            "HTTPS://EXAMPLE.COM",
            "https://example.com/path?query=1#fragment",
            "https://example.com:8443/artifact",
        ] {
            assert!(
                validate_link_entry("links", link).is_ok(),
                "{link} must be accepted"
            );
        }
    }

    #[test]
    fn link_safety_rejects_unsafe_schemes_and_credentials() {
        for link in [
            "javascript:alert(1)",
            "javascript://example.com",
            "data:text/html;base64,AAAA",
            "file:///etc/passwd",
            "ftp://example.com",
            "http:/example.com",
            "//example.com",
            "example.com/artifact",
            "http://",
            "http://user:secret@example.com",
            "http://user@example.com",
            "https://example.com/with space",
            "https://example.com/with\nnewline",
            "",
        ] {
            let err = validate_link_entry("links", link).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation { field: "links", .. }),
                "{link:?} must be rejected"
            );
        }
    }

    #[test]
    fn link_safety_credentials_after_authority_are_allowed() {
        // '@' in the path or query is not userinfo.
        for link in [
            "https://example.com/users/@alice",
            "https://example.com/search?q=a@b",
        ] {
            assert!(
                validate_link_entry("links", link).is_ok(),
                "{link} must be accepted"
            );
        }
    }
}
