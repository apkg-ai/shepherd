use crate::storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("resource not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: i64, actual: i64 },
    #[error("expected revision required for this command")]
    PreconditionRequired,
    #[error("validation failed on {field}: {message}")]
    Validation {
        field: &'static str,
        message: String,
    },
    #[error("invalid cursor: {0}")]
    InvalidCursor(String),
    #[error("duplicate task type key: {key}")]
    DuplicateTaskTypeKey { key: String },
    #[error("resource is archived")]
    ArchivedScope,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for DomainError {
    fn from(err: sqlx::Error) -> Self {
        DomainError::Storage(StorageError::from(err))
    }
}

impl DomainError {
    /// Stable Problem codes (plan/07); the server maps them to wire responses in step 015.
    pub fn code(&self) -> &'static str {
        match self {
            DomainError::NotFound => "not_found",
            DomainError::Forbidden(_) => "forbidden",
            DomainError::RevisionConflict { .. } => "revision_conflict",
            DomainError::PreconditionRequired => "precondition_required",
            DomainError::Validation { .. } => "validation_error",
            DomainError::InvalidCursor(_) => "invalid_cursor",
            // Wire code undecided until step 015; plan/07's 409 list has no duplicate-key entry.
            DomainError::DuplicateTaskTypeKey { .. } => "validation_error",
            DomainError::ArchivedScope => "terminal",
            DomainError::Storage(_) => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_problem_identifiers() {
        assert_eq!(DomainError::NotFound.code(), "not_found");
        assert_eq!(DomainError::Forbidden("agent".into()).code(), "forbidden");
        assert_eq!(
            DomainError::RevisionConflict {
                expected: 2,
                actual: 1
            }
            .code(),
            "revision_conflict"
        );
        assert_eq!(
            DomainError::PreconditionRequired.code(),
            "precondition_required"
        );
        assert_eq!(
            DomainError::Validation {
                field: "name",
                message: "blank".into()
            }
            .code(),
            "validation_error"
        );
        assert_eq!(
            DomainError::InvalidCursor("mangled".into()).code(),
            "invalid_cursor"
        );
        assert_eq!(DomainError::ArchivedScope.code(), "terminal");
    }

    #[test]
    fn storage_errors_convert_transparently() {
        let err = DomainError::from(StorageError::Corrupt("bad row".into()));
        assert!(matches!(
            err,
            DomainError::Storage(StorageError::Corrupt(_))
        ));
        assert_eq!(err.code(), "internal");
    }
}
