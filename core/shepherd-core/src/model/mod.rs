mod epic;
mod goal;
mod project;
mod task;

pub use epic::{Epic, EpicCreate, EpicId, EpicStatus};
pub use goal::{Goal, GoalCreate, GoalId, TextPatch, goal_completed};
pub use project::{
    BUILTIN_TASK_TYPES, Project, ProjectCreate, ProjectId, ProjectPatch, ProjectSettings,
    ReviewPolicy, TaskType, TaskTypeCreate, TaskTypeId, TaskTypePatch,
};
pub(crate) use project::{
    DESCRIPTION_MAX_CHARS, NAME_MAX_CHARS, validate_type_key, validate_type_label,
};
pub use task::{Task, TaskCreate, TaskId, TaskPatch, TaskPhase, TaskStatus};

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use chrono::{DateTime, SubsecRound, Utc};
use uuid::timestamp::context::ContextV7;
use uuid::{Timestamp, Uuid};

use crate::error::DomainError;

macro_rules! typed_uuid {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            pub fn generate(now: DateTime<Utc>) -> Self {
                Self(crate::model::new_v7(now))
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(value)?))
            }
        }
    };
}

pub(crate) use typed_uuid;

macro_rules! resource_status {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            Proposed,
            Open,
            Active,
            Done,
            Cancelled,
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $name::Proposed => "proposed",
                    $name::Open => "open",
                    $name::Active => "active",
                    $name::Done => "done",
                    $name::Cancelled => "cancelled",
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    "proposed" => Some($name::Proposed),
                    "open" => Some($name::Open),
                    "active" => Some($name::Active),
                    "done" => Some($name::Done),
                    "cancelled" => Some($name::Cancelled),
                    _ => None,
                }
            }

            pub fn is_terminal(self) -> bool {
                matches!(self, $name::Done | $name::Cancelled)
            }
        }
    };
}

pub(crate) use resource_status;

typed_uuid!(ActorId);
typed_uuid!(CommandId);
typed_uuid!(DependencyId);

// Graph nodes carry one shared status vocabulary across epic and task levels.
resource_status!(EntityStatus);

impl From<EpicStatus> for EntityStatus {
    fn from(status: EpicStatus) -> Self {
        match status {
            EpicStatus::Proposed => EntityStatus::Proposed,
            EpicStatus::Open => EntityStatus::Open,
            EpicStatus::Active => EntityStatus::Active,
            EpicStatus::Done => EntityStatus::Done,
            EpicStatus::Cancelled => EntityStatus::Cancelled,
        }
    }
}

impl From<TaskStatus> for EntityStatus {
    fn from(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Proposed => EntityStatus::Proposed,
            TaskStatus::Open => EntityStatus::Open,
            TaskStatus::Active => EntityStatus::Active,
            TaskStatus::Done => EntityStatus::Done,
            TaskStatus::Cancelled => EntityStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyLevel {
    Epic,
    Task,
}

impl DependencyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            DependencyLevel::Epic => "epic",
            DependencyLevel::Task => "task",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "epic" => Some(DependencyLevel::Epic),
            "task" => Some(DependencyLevel::Task),
            _ => None,
        }
    }
}

// Endpoints stay untyped Uuids: the wire model is level-discriminated; typed
// safety lives in DependencyCreate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: DependencyId,
    pub revision: Revision,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_id: ProjectId,
    pub level: DependencyLevel,
    pub dependent_id: Uuid,
    pub prerequisite_id: Uuid,
}

/// One typed command payload over the two dependency tables (plan/03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyCreate {
    Epic {
        dependent_id: EpicId,
        prerequisite_id: EpicId,
    },
    Task {
        dependent_id: TaskId,
        prerequisite_id: TaskId,
    },
}

impl DependencyCreate {
    pub fn level(&self) -> DependencyLevel {
        match self {
            DependencyCreate::Epic { .. } => DependencyLevel::Epic,
            DependencyCreate::Task { .. } => DependencyLevel::Task,
        }
    }
}

// ContextV7 is not Sync; the shared context keeps same-millisecond IDs monotonic.
static V7_CONTEXT: LazyLock<std::sync::Mutex<ContextV7>> =
    LazyLock::new(|| std::sync::Mutex::new(ContextV7::new()));

pub(crate) fn new_v7(now: DateTime<Utc>) -> Uuid {
    let seconds = u64::try_from(now.timestamp()).unwrap_or(0);
    let context = V7_CONTEXT.lock().unwrap();
    Uuid::new_v7(Timestamp::from_unix(
        &*context,
        seconds,
        now.timestamp_subsec_nanos(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(i64);

impl Revision {
    pub const INITIAL: Revision = Revision(1);

    pub fn from_stored(value: i64) -> Option<Self> {
        (value >= 1).then_some(Self(value))
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn value(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Agent,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Human => "human",
            ActorKind::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(ActorKind::Human),
            "agent" => Some(ActorKind::Agent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub id: ActorId,
    pub kind: ActorKind,
    pub label: String,
    pub revoked: bool,
    pub created_at: DateTime<Utc>,
}

/// events.id is the SQLite rowid.
pub type EventId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub total: i64,
    pub done: i64,
    pub cancelled: i64,
    pub waived: i64,
}

impl Counts {
    pub const ZERO: Counts = Counts {
        total: 0,
        done: 0,
        cancelled: 0,
        waived: 0,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRecord {
    pub actor_id: ActorId,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

// Contract maxLength counts code points, so validators count chars, not bytes.
pub(crate) fn validate_required_text(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation {
            field,
            message: "must not be blank".into(),
        });
    }
    if trimmed.chars().count() > max_chars {
        return Err(DomainError::Validation {
            field,
            message: format!("must be at most {max_chars} characters"),
        });
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_long_text(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<(), DomainError> {
    if value.chars().count() > max_chars {
        return Err(DomainError::Validation {
            field,
            message: format!("must be at most {max_chars} characters"),
        });
    }
    Ok(())
}

pub trait Clock: Send + Sync {
    /// Millisecond precision so stored RFC 3339 values round-trip exactly.
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(3)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestClock(std::sync::Mutex<DateTime<Utc>>);

#[cfg(any(test, feature = "test-support"))]
impl TestClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self(std::sync::Mutex::new(start.trunc_subsecs(3)))
    }

    pub fn set(&self, now: DateTime<Utc>) {
        *self.0.lock().unwrap() = now.trunc_subsecs(3);
    }

    pub fn advance(&self, delta: chrono::TimeDelta) {
        let mut guard = self.0.lock().unwrap();
        *guard = (*guard + delta).trunc_subsecs(3);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    #[test]
    fn generated_ids_are_uuid_v7_ordered_by_time() {
        let earlier = ActorId::generate(ts("2026-09-14T00:00:00Z"));
        let later = ActorId::generate(ts("2026-09-15T00:00:00Z"));
        assert_eq!(earlier.as_uuid().get_version_num(), 7);
        assert!(earlier < later);
    }

    #[test]
    fn typed_ids_round_trip_through_text() {
        let id = CommandId::generate(ts("2026-09-14T00:00:00Z"));
        let parsed: CommandId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
        assert!("not-a-uuid".parse::<CommandId>().is_err());
    }

    #[test]
    fn pre_epoch_timestamps_do_not_panic() {
        let id = ActorId::generate(ts("1969-01-01T00:00:00Z"));
        assert_eq!(id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn same_millisecond_ids_are_monotonic() {
        let now = ts("2026-09-14T00:00:00Z");
        let first = ActorId::generate(now);
        let second = ActorId::generate(now);
        let third = ActorId::generate(now);
        assert!(first < second);
        assert!(second < third);
    }

    #[test]
    fn revision_starts_at_one_and_increments() {
        assert_eq!(Revision::INITIAL.value(), 1);
        assert_eq!(Revision::INITIAL.next().value(), 2);
        assert_eq!(
            Revision::from_stored(3),
            Some(Revision::INITIAL.next().next())
        );
        assert_eq!(Revision::from_stored(0), None);
        assert_eq!(Revision::from_stored(-1), None);
    }

    #[test]
    fn dependency_level_round_trips_stored_strings() {
        for level in [DependencyLevel::Epic, DependencyLevel::Task] {
            assert_eq!(DependencyLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(DependencyLevel::parse("goal"), None);
    }

    #[test]
    fn dependency_create_reports_its_level() {
        let now = ts("2026-09-14T00:00:00Z");
        let epic_link = DependencyCreate::Epic {
            dependent_id: EpicId::generate(now),
            prerequisite_id: EpicId::generate(now),
        };
        assert_eq!(epic_link.level(), DependencyLevel::Epic);
        let task_link = DependencyCreate::Task {
            dependent_id: TaskId::generate(now),
            prerequisite_id: TaskId::generate(now),
        };
        assert_eq!(task_link.level(), DependencyLevel::Task);
    }

    #[test]
    fn entity_status_converts_from_both_levels() {
        assert_eq!(
            EntityStatus::from(EpicStatus::Proposed).as_str(),
            "proposed"
        );
        assert_eq!(EntityStatus::from(EpicStatus::Open).as_str(), "open");
        assert_eq!(EntityStatus::from(EpicStatus::Active).as_str(), "active");
        assert_eq!(EntityStatus::from(EpicStatus::Done).as_str(), "done");
        assert_eq!(
            EntityStatus::from(EpicStatus::Cancelled).as_str(),
            "cancelled"
        );
        assert_eq!(
            EntityStatus::from(TaskStatus::Proposed).as_str(),
            "proposed"
        );
        assert_eq!(EntityStatus::from(TaskStatus::Open).as_str(), "open");
        assert_eq!(EntityStatus::from(TaskStatus::Active).as_str(), "active");
        assert_eq!(EntityStatus::from(TaskStatus::Done).as_str(), "done");
        assert_eq!(
            EntityStatus::from(TaskStatus::Cancelled).as_str(),
            "cancelled"
        );
    }

    #[test]
    fn actor_kind_round_trips_stored_strings() {
        for kind in [ActorKind::Human, ActorKind::Agent] {
            assert_eq!(ActorKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ActorKind::parse("owner"), None);
    }

    #[test]
    fn system_clock_truncates_to_milliseconds() {
        let now = SystemClock.now();
        assert_eq!(now.timestamp_subsec_nanos() % 1_000_000, 0);
    }

    #[test]
    fn test_clock_is_controllable() {
        let clock = TestClock::new(ts("2026-09-14T00:00:00Z"));
        assert_eq!(clock.now(), ts("2026-09-14T00:00:00Z"));
        clock.advance(chrono::TimeDelta::seconds(90));
        assert_eq!(clock.now(), ts("2026-09-14T00:01:30Z"));
        clock.set(ts("2026-09-14T12:00:00.5Z"));
        assert_eq!(clock.now(), ts("2026-09-14T12:00:00.500Z"));
    }

    #[test]
    fn test_clock_advance_keeps_millisecond_precision() {
        let clock = TestClock::new(ts("2026-09-14T00:00:00Z"));
        clock.advance(chrono::TimeDelta::microseconds(1500));
        assert_eq!(clock.now(), ts("2026-09-14T00:00:00.001Z"));
        assert_eq!(clock.now().timestamp_subsec_nanos() % 1_000_000, 0);
    }
}
