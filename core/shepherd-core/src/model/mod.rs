use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use chrono::{DateTime, SubsecRound, Utc};
use uuid::timestamp::context::ContextV7;
use uuid::{Timestamp, Uuid};

macro_rules! typed_uuid {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            pub fn generate(now: DateTime<Utc>) -> Self {
                Self(new_v7(now))
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

typed_uuid!(ActorId);
typed_uuid!(CommandId);

// Shared ContextV7 keeps IDs generated within one millisecond monotonically
// ordered; it is not Sync, hence the mutex.
static V7_CONTEXT: LazyLock<std::sync::Mutex<ContextV7>> =
    LazyLock::new(|| std::sync::Mutex::new(ContextV7::new()));

fn new_v7(now: DateTime<Utc>) -> Uuid {
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
