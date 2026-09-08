//! Lease math for task claims.
//!
//! Pure functions — no storage access. The store calls these for expiry
//! checking, TTL computation, and lease ID generation.

use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::error::Error;

/// Minimum TTL in seconds.
pub const MIN_TTL: i32 = 30;
/// Maximum TTL in seconds (24 hours).
pub const MAX_TTL: i32 = 86400;

/// Check whether a claim is expired at the given time.
pub fn is_expired(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now >= expires_at
}

/// Compute `expires_at` from acquisition time and TTL.
pub fn compute_expiry(acquired_at: DateTime<Utc>, ttl_seconds: i32) -> DateTime<Utc> {
    acquired_at + TimeDelta::seconds(i64::from(ttl_seconds))
}

/// Compute `expires_at` from renewal time and TTL.
pub fn compute_renewal_expiry(now: DateTime<Utc>, ttl_seconds: i32) -> DateTime<Utc> {
    now + TimeDelta::seconds(i64::from(ttl_seconds))
}

/// Generate an opaque lease ID (UUID-based string, no dashes).
pub fn generate_lease_id() -> String {
    Uuid::now_v7().simple().to_string()
}

/// Validate that TTL is within the allowed range.
pub fn validate_ttl(ttl_seconds: i32) -> Result<(), Error> {
    if !(MIN_TTL..=MAX_TTL).contains(&ttl_seconds) {
        return Err(Error::ValidationError {
            detail: format!(
                "ttl_seconds must be between {MIN_TTL} and {MAX_TTL}, got {ttl_seconds}"
            ),
            errors: vec![],
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_expired_before_deadline() {
        let now = Utc::now();
        let expires = now + TimeDelta::seconds(60);
        assert!(!is_expired(expires, now));
    }

    #[test]
    fn expired_at_deadline() {
        let now = Utc::now();
        assert!(is_expired(now, now));
    }

    #[test]
    fn expired_after_deadline() {
        let now = Utc::now();
        let expires = now - TimeDelta::seconds(1);
        assert!(is_expired(expires, now));
    }

    #[test]
    fn compute_expiry_adds_ttl() {
        let acquired = Utc::now();
        let expires = compute_expiry(acquired, 300);
        let diff = (expires - acquired).num_seconds();
        assert_eq!(diff, 300);
    }

    #[test]
    fn compute_renewal_expiry_from_now() {
        let now = Utc::now();
        let expires = compute_renewal_expiry(now, 600);
        let diff = (expires - now).num_seconds();
        assert_eq!(diff, 600);
    }

    #[test]
    fn lease_id_is_nonempty() {
        let id = generate_lease_id();
        assert!(!id.is_empty());
        assert!(!id.contains('-'));
    }

    #[test]
    fn lease_ids_are_unique() {
        let a = generate_lease_id();
        let b = generate_lease_id();
        assert_ne!(a, b);
    }

    #[test]
    fn validate_ttl_in_range() {
        assert!(validate_ttl(MIN_TTL).is_ok());
        assert!(validate_ttl(MAX_TTL).is_ok());
        assert!(validate_ttl(3600).is_ok());
    }

    #[test]
    fn validate_ttl_out_of_range() {
        assert!(validate_ttl(MIN_TTL - 1).is_err());
        assert!(validate_ttl(MAX_TTL + 1).is_err());
        assert!(validate_ttl(0).is_err());
        assert!(validate_ttl(-1).is_err());
    }
}
