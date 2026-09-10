//! shepherd domain core.
//!
//! Owns entities, the lifecycle state machine, DAG operations, lease logic,
//! context-bundle assembly, and storage. No HTTP types leak into this crate.

pub mod bundle;
pub mod dag;
pub mod error;
pub mod event;
pub mod export;
pub mod lease;
pub mod lifecycle;
pub mod model;
pub mod store;

pub use error::Error;
pub use event::{DomainEvent, EventBus};
pub use model::*;
pub use store::Store;

/// Convenience alias for domain results.
pub type Result<T> = std::result::Result<T, Error>;

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
