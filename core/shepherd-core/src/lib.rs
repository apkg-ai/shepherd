//! shepherd domain core.
//!
//! Owns entities, the lifecycle state machine, DAG operations, lease logic,
//! context-bundle assembly, and storage. No HTTP types leak into this crate.
//! The domain lands in S3 ([06-roadmap](../../docs/06-roadmap.md)); S1 ships
//! the crate skeleton.

/// The shepherd-core version, embedded at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_crate_metadata() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }
}
