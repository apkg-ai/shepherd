//! Domain errors with RFC 9457 URN-slug mapping.
//!
//! Every variant maps to a stable `urn:shepherd:error:*` type slug, an HTTP
//! status code, and a human-readable title. The server layer translates these
//! into `application/problem+json` responses.

use crate::model::TaskStatus;

/// A single field-level validation error.
#[derive(Debug, Clone)]
pub struct ValidationFieldError {
    /// JSON Pointer to the invalid field (e.g. `/title`).
    pub field: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional machine-readable error code.
    pub code: Option<String>,
}

/// Domain error type. Each variant corresponds to a documented
/// `urn:shepherd:error:*` slug from the OpenAPI spec.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {detail}")]
    NotFound { detail: String },

    #[error("validation error: {detail}")]
    ValidationError {
        detail: String,
        errors: Vec<ValidationFieldError>,
    },

    #[error("dependency cycle: {detail}")]
    DependencyCycle { detail: String },

    #[error("claim conflict: {detail}")]
    ClaimConflict { detail: String },

    #[error("invalid transition from {from}: {detail}")]
    InvalidTransition {
        from: TaskStatus,
        trigger: String,
        detail: String,
    },

    #[error("lease expired: {detail}")]
    LeaseExpired { detail: String },

    #[error("task not ready: {detail}")]
    TaskNotReady { detail: String },

    #[error("decomposition violation: {detail}")]
    DecompositionViolation { detail: String },

    #[error("import schema mismatch: {detail}")]
    ImportSchemaMismatch { detail: String },

    #[error("internal error: {0}")]
    Internal(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl Error {
    /// Stable URN type slug for machine-readable error identification.
    pub fn urn(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "urn:shepherd:error:not-found",
            Self::ValidationError { .. } => "urn:shepherd:error:validation-error",
            Self::DependencyCycle { .. } => "urn:shepherd:error:dependency-cycle",
            Self::ClaimConflict { .. } => "urn:shepherd:error:claim-conflict",
            Self::InvalidTransition { .. } => "urn:shepherd:error:invalid-transition",
            Self::LeaseExpired { .. } => "urn:shepherd:error:lease-expired",
            Self::TaskNotReady { .. } => "urn:shepherd:error:task-not-ready",
            Self::DecompositionViolation { .. } => "urn:shepherd:error:decomposition-violation",
            Self::ImportSchemaMismatch { .. } => "urn:shepherd:error:import-schema-mismatch",
            Self::Internal(_) => "urn:shepherd:error:internal-error",
            Self::Database(_) => "urn:shepherd:error:internal-error",
        }
    }

    /// HTTP status code that the server should return.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotFound { .. } => 404,
            Self::ValidationError { .. } => 422,
            Self::DependencyCycle { .. } => 409,
            Self::ClaimConflict { .. } => 409,
            Self::InvalidTransition { .. } => 409,
            Self::LeaseExpired { .. } => 410,
            Self::TaskNotReady { .. } => 409,
            Self::DecompositionViolation { .. } => 409,
            Self::ImportSchemaMismatch { .. } => 422,
            Self::Internal(_) => 500,
            Self::Database(_) => 500,
        }
    }

    /// Short human-readable title for the error category.
    pub fn title(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "Not Found",
            Self::ValidationError { .. } => "Validation Error",
            Self::DependencyCycle { .. } => "Dependency Cycle",
            Self::ClaimConflict { .. } => "Claim Conflict",
            Self::InvalidTransition { .. } => "Invalid Transition",
            Self::LeaseExpired { .. } => "Lease Expired",
            Self::TaskNotReady { .. } => "Task Not Ready",
            Self::DecompositionViolation { .. } => "Decomposition Violation",
            Self::ImportSchemaMismatch { .. } => "Import Schema Mismatch",
            Self::Internal(_) => "Internal Error",
            Self::Database(_) => "Internal Error",
        }
    }

    /// Shorthand for a not-found error.
    pub fn not_found(entity: &str, id: impl std::fmt::Display) -> Self {
        Self::NotFound {
            detail: format!("{entity} {id} not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urn_mapping_is_stable() {
        let err = Error::not_found("project", "abc");
        assert_eq!(err.urn(), "urn:shepherd:error:not-found");
        assert_eq!(err.status_code(), 404);
        assert_eq!(err.title(), "Not Found");
    }

    #[test]
    fn all_variants_have_valid_urns() {
        let variants: Vec<Error> = vec![
            Error::NotFound {
                detail: String::new(),
            },
            Error::ValidationError {
                detail: String::new(),
                errors: vec![],
            },
            Error::DependencyCycle {
                detail: String::new(),
            },
            Error::ClaimConflict {
                detail: String::new(),
            },
            Error::InvalidTransition {
                from: TaskStatus::Proposed,
                trigger: String::new(),
                detail: String::new(),
            },
            Error::LeaseExpired {
                detail: String::new(),
            },
            Error::TaskNotReady {
                detail: String::new(),
            },
            Error::DecompositionViolation {
                detail: String::new(),
            },
            Error::ImportSchemaMismatch {
                detail: String::new(),
            },
            Error::Internal(String::new()),
        ];

        for err in &variants {
            assert!(
                err.urn().starts_with("urn:shepherd:error:"),
                "bad URN for {}",
                err
            );
            assert!(
                (400..=599).contains(&err.status_code()),
                "bad status for {}",
                err
            );
            assert!(!err.title().is_empty(), "empty title for {}", err);
        }
    }
}
