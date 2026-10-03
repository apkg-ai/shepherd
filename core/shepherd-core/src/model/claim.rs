use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ActorId, TaskId};
use crate::error::DomainError;

pub const DEFAULT_TTL_SECONDS: i64 = 300;
pub const MIN_TTL_SECONDS: i64 = 30;
pub const MAX_TTL_SECONDS: i64 = 900;

// Neither Uuid nor DateTime<Utc> derive Serialize/Deserialize in this crate
// (no `serde` feature on the uuid or chrono dependencies). These helpers
// serialise through the string representation so idempotent-replay
// round-trips work.
mod serde_datetime {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        crate::storage::rows::format_ts(dt).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        crate::storage::rows::parse_ts("claim.datetime", &text).map_err(serde::de::Error::custom)
    }
}

mod serde_uuid {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    pub fn serialize<S: Serializer>(id: &Uuid, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Uuid, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text).map_err(serde::de::Error::custom)
    }
}

mod serde_uuid_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    pub fn serialize<S: Serializer>(ids: &[Uuid], s: S) -> Result<S::Ok, S::Error> {
        ids.iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Uuid>, D::Error> {
        let strings: Vec<String> = Vec::deserialize(d)?;
        strings
            .iter()
            .map(|s| Uuid::parse_str(s).map_err(serde::de::Error::custom))
            .collect()
    }
}

mod serde_task_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    use crate::model::TaskId;

    pub fn serialize<S: Serializer>(id: &TaskId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<TaskId, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text)
            .map(TaskId::from_uuid)
            .map_err(serde::de::Error::custom)
    }
}

mod serde_actor_id {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use uuid::Uuid;

    use crate::model::ActorId;

    pub fn serialize<S: Serializer>(id: &ActorId, s: S) -> Result<S::Ok, S::Error> {
        id.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ActorId, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text)
            .map(ActorId::from_uuid)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimPhase {
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "execute")]
    Execute,
    #[serde(rename = "review")]
    Review,
}

impl ClaimPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimPhase::Plan => "plan",
            ClaimPhase::Execute => "execute",
            ClaimPhase::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plan" => Some(ClaimPhase::Plan),
            "execute" => Some(ClaimPhase::Execute),
            "review" => Some(ClaimPhase::Review),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "released")]
    Released,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "revoked")]
    Revoked,
    #[serde(rename = "reported")]
    Reported,
}

impl ClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimStatus::Active => "active",
            ClaimStatus::Released => "released",
            ClaimStatus::Expired => "expired",
            ClaimStatus::Revoked => "revoked",
            ClaimStatus::Reported => "reported",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(ClaimStatus::Active),
            "released" => Some(ClaimStatus::Released),
            "expired" => Some(ClaimStatus::Expired),
            "revoked" => Some(ClaimStatus::Revoked),
            "reported" => Some(ClaimStatus::Reported),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    #[serde(with = "serde_task_id")]
    pub task_id: TaskId,
    #[serde(with = "serde_actor_id")]
    pub actor_id: ActorId,
    pub phase: ClaimPhase,
    #[serde(with = "serde_uuid_vec")]
    pub submission_ids: Vec<Uuid>,
    #[serde(with = "serde_datetime")]
    pub acquired_at: DateTime<Utc>,
    #[serde(with = "serde_datetime")]
    pub expires_at: DateTime<Utc>,
    pub status: ClaimStatus,
    pub task_revision: i64,
    #[serde(with = "serde_uuid_vec")]
    pub plan_revision_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimGrant {
    pub claim: Claim,
    pub lease_token: String,
}

#[derive(Debug, Clone)]
pub struct ClaimInput {
    pub phase: ClaimPhase,
    pub submission_id: Option<Uuid>,
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RenewInput {
    pub ttl_seconds: Option<i64>,
}

pub fn validate_ttl(ttl_seconds: Option<i64>) -> Result<i64, DomainError> {
    let ttl = ttl_seconds.unwrap_or(DEFAULT_TTL_SECONDS);
    if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl) {
        return Err(DomainError::Validation {
            field: "ttl_seconds",
            message: format!("must be between {MIN_TTL_SECONDS} and {MAX_TTL_SECONDS}"),
        });
    }
    Ok(ttl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_phase_round_trips_stored_strings() {
        for phase in [ClaimPhase::Plan, ClaimPhase::Execute, ClaimPhase::Review] {
            assert_eq!(ClaimPhase::parse(phase.as_str()), Some(phase));
        }
        assert_eq!(ClaimPhase::parse("planning"), None);
    }

    #[test]
    fn claim_status_round_trips_stored_strings() {
        for status in [
            ClaimStatus::Active,
            ClaimStatus::Released,
            ClaimStatus::Expired,
            ClaimStatus::Revoked,
            ClaimStatus::Reported,
        ] {
            assert_eq!(ClaimStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ClaimStatus::parse("pending"), None);
    }

    #[test]
    fn validate_ttl_defaults_and_rejects_out_of_range() {
        assert_eq!(validate_ttl(None).unwrap(), 300);
        assert_eq!(validate_ttl(Some(30)).unwrap(), 30);
        assert_eq!(validate_ttl(Some(900)).unwrap(), 900);
        assert!(validate_ttl(Some(29)).is_err());
        assert!(validate_ttl(Some(901)).is_err());
    }

    #[test]
    fn claim_phase_serde_uses_wire_vocabulary() {
        let json = serde_json::to_string(&ClaimPhase::Plan).unwrap();
        assert_eq!(json, r#""plan""#);
        let parsed: ClaimPhase = serde_json::from_str(r#""execute""#).unwrap();
        assert_eq!(parsed, ClaimPhase::Execute);
    }

    #[test]
    fn claim_status_serde_uses_wire_vocabulary() {
        let json = serde_json::to_string(&ClaimStatus::Active).unwrap();
        assert_eq!(json, r#""active""#);
        let parsed: ClaimStatus = serde_json::from_str(r#""revoked""#).unwrap();
        assert_eq!(parsed, ClaimStatus::Revoked);
    }

    #[test]
    fn claim_round_trips_through_json() {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        let claim = Claim {
            id: Uuid::nil(),
            task_id: TaskId::from_uuid(Uuid::nil()),
            actor_id: ActorId::from_uuid(Uuid::nil()),
            phase: ClaimPhase::Plan,
            submission_ids: Vec::new(),
            acquired_at: now,
            expires_at: now,
            status: ClaimStatus::Active,
            task_revision: 1,
            plan_revision_ids: Vec::new(),
        };
        let json = serde_json::to_string(&claim).unwrap();
        let parsed: Claim = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, claim);
    }
}
