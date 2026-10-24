use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::claim::{serde_actor_id, serde_datetime, serde_uuid, serde_uuid_vec};
use super::content::serde_project_id;
use super::{ActorId, CommandId, ProjectId, typed_id_serde};

/// events.id is the SQLite rowid.
pub type EventId = i64;

typed_id_serde!(serde_command_id, CommandId);

// Wire ids are decimal strings (`^[1-9][0-9]*$`): JS numbers lose precision
// past 2^53, so the contract never exposes the rowid as a JSON number.
mod serde_event_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::EventId;

    pub fn serialize<S: Serializer>(id: &EventId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<EventId, D::Error> {
        let text = String::deserialize(d)?;
        let id: EventId = text
            .parse()
            .map_err(|_| serde::de::Error::custom("event id must be a decimal integer"))?;
        // Round-trip equality rejects leading zeros and signs; ids start at 1.
        if id < 1 || id.to_string() != text {
            return Err(serde::de::Error::custom(
                "event id must match ^[1-9][0-9]*$",
            ));
        }
        Ok(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "project.changed")]
    ProjectChanged,
    #[serde(rename = "goal.changed")]
    GoalChanged,
    #[serde(rename = "epic.changed")]
    EpicChanged,
    #[serde(rename = "task.changed")]
    TaskChanged,
    #[serde(rename = "dependency.changed")]
    DependencyChanged,
    #[serde(rename = "claim.changed")]
    ClaimChanged,
    #[serde(rename = "session.recorded")]
    SessionRecorded,
    #[serde(rename = "document.changed")]
    DocumentChanged,
    #[serde(rename = "submission.changed")]
    SubmissionChanged,
    #[serde(rename = "review.recorded")]
    ReviewRecorded,
    #[serde(rename = "task_type.changed")]
    TaskTypeChanged,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::ProjectChanged => "project.changed",
            EventType::GoalChanged => "goal.changed",
            EventType::EpicChanged => "epic.changed",
            EventType::TaskChanged => "task.changed",
            EventType::DependencyChanged => "dependency.changed",
            EventType::ClaimChanged => "claim.changed",
            EventType::SessionRecorded => "session.recorded",
            EventType::DocumentChanged => "document.changed",
            EventType::SubmissionChanged => "submission.changed",
            EventType::ReviewRecorded => "review.recorded",
            EventType::TaskTypeChanged => "task_type.changed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project.changed" => Some(EventType::ProjectChanged),
            "goal.changed" => Some(EventType::GoalChanged),
            "epic.changed" => Some(EventType::EpicChanged),
            "task.changed" => Some(EventType::TaskChanged),
            "dependency.changed" => Some(EventType::DependencyChanged),
            "claim.changed" => Some(EventType::ClaimChanged),
            "session.recorded" => Some(EventType::SessionRecorded),
            "document.changed" => Some(EventType::DocumentChanged),
            "submission.changed" => Some(EventType::SubmissionChanged),
            "review.recorded" => Some(EventType::ReviewRecorded),
            "task_type.changed" => Some(EventType::TaskTypeChanged),
            _ => None,
        }
    }
}

/// Read model over persisted event rows (plan/08); emission lives in commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    #[serde(with = "serde_event_id")]
    pub id: EventId,
    #[serde(with = "serde_project_id")]
    pub project_id: ProjectId,
    #[serde(with = "serde_actor_id")]
    pub actor_id: ActorId,
    #[serde(with = "serde_command_id")]
    pub command_id: CommandId,
    #[serde(rename = "type")]
    pub event_type: EventType,
    #[serde(with = "serde_uuid")]
    pub resource_id: Uuid,
    pub resource_revision: i64,
    #[serde(with = "serde_datetime")]
    pub occurred_at: DateTime<Utc>,
    #[serde(with = "serde_uuid_vec")]
    pub affected_ids: Vec<Uuid>,
    pub action: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Event {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        Event {
            id: 42,
            project_id: ProjectId::from_uuid(Uuid::nil()),
            actor_id: ActorId::from_uuid(Uuid::nil()),
            command_id: CommandId::from_uuid(Uuid::nil()),
            event_type: EventType::TaskChanged,
            resource_id: Uuid::nil(),
            resource_revision: 3,
            occurred_at: now,
            affected_ids: vec![Uuid::nil()],
            action: "updateTask".into(),
            reason: "".into(),
        }
    }

    #[test]
    fn event_round_trips_through_json() {
        let event = sample();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn event_id_serializes_as_decimal_string_and_type_as_wire_key() {
        let value = serde_json::to_value(sample()).unwrap();
        assert_eq!(value["id"], serde_json::json!("42"));
        assert_eq!(value["type"], serde_json::json!("task.changed"));
    }

    #[test]
    fn event_id_rejects_non_positive_and_padded_strings() {
        for bad in ["0", "-1", "01", "+2", "abc", ""] {
            let json = serde_json::to_string(&sample())
                .unwrap()
                .replace("\"42\"", &format!("\"{bad}\""));
            assert!(serde_json::from_str::<Event>(&json).is_err(), "{bad}");
        }
    }

    #[test]
    fn event_type_round_trips_stored_strings() {
        for name in [
            "project.changed",
            "goal.changed",
            "epic.changed",
            "task.changed",
            "dependency.changed",
            "claim.changed",
            "session.recorded",
            "document.changed",
            "submission.changed",
            "review.recorded",
            "task_type.changed",
        ] {
            let parsed = EventType::parse(name).unwrap();
            assert_eq!(parsed.as_str(), name);
        }
        assert_eq!(EventType::parse("claim.created"), None);
    }
}
