//! shepherd domain core.
//!
//! Reduced to an importable crate shell by v1 step 000 (see
//! `plan/steps/000-scaffold-reset.md`). Later steps rebuild the stable-v1
//! domain here. No HTTP types leak into this crate.

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
