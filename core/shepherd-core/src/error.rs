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
    #[error("resource or scope is terminal")]
    TerminalScope,
    #[error("invalid state transition: {0}")]
    InvalidState(String),
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
            DomainError::TerminalScope => "terminal",
            DomainError::InvalidState(_) => "invalid_state",
            DomainError::Storage(StorageError::Corrupt(_)) => "integrity_failure",
            // SQLITE_BUSY arrives as the low byte of the sqlite extended code.
            DomainError::Storage(StorageError::Sqlx(sqlx::Error::Database(db)))
                if is_sqlite_busy(db.as_ref()) =>
            {
                "storage_busy"
            }
            DomainError::Storage(_) => "internal_error",
        }
    }
}

const SQLITE_BUSY: i64 = 5;

// SqliteError::code() renders sqlite3_extended_errcode as a string (sqlx-sqlite 0.9).
fn is_sqlite_busy(db: &dyn sqlx::error::DatabaseError) -> bool {
    db.code()
        .as_deref()
        .and_then(|code| code.parse::<i64>().ok())
        .is_some_and(|extended| extended & 0xFF == SQLITE_BUSY)
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
        assert_eq!(DomainError::TerminalScope.code(), "terminal");
        assert_eq!(
            DomainError::InvalidState("wrong".into()).code(),
            "invalid_state"
        );
    }

    #[test]
    fn storage_errors_convert_transparently() {
        let err = DomainError::from(StorageError::Corrupt("bad row".into()));
        assert!(matches!(
            err,
            DomainError::Storage(StorageError::Corrupt(_))
        ));
        assert_eq!(err.code(), "integrity_failure");
    }

    #[test]
    fn storage_wire_codes_follow_plan_seven() {
        assert_eq!(
            DomainError::Storage(StorageError::MvpDatabase {
                path: "mvp/shepherd.db".into()
            })
            .code(),
            "internal_error"
        );
        assert_eq!(
            DomainError::Storage(StorageError::Codec("boom".into())).code(),
            "internal_error"
        );
    }

    // SqliteError has no public constructor; the fake carries just the code string.
    struct FakeDatabaseError(&'static str);

    impl std::fmt::Debug for FakeDatabaseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::fmt::Display for FakeDatabaseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for FakeDatabaseError {}

    impl sqlx::error::DatabaseError for FakeDatabaseError {
        fn message(&self) -> &str {
            self.0
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(self.0.into())
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    #[test]
    fn busy_sqlite_codes_map_to_storage_busy() {
        for extended in ["5", "517"] {
            let err = DomainError::Storage(StorageError::Sqlx(sqlx::Error::Database(Box::new(
                FakeDatabaseError(extended),
            ))));
            assert_eq!(err.code(), "storage_busy", "extended code {extended}");
        }
        for other in ["6", "1", "not numeric"] {
            let err = DomainError::Storage(StorageError::Sqlx(sqlx::Error::Database(Box::new(
                FakeDatabaseError(other),
            ))));
            assert_eq!(err.code(), "internal_error", "code {other}");
        }
        let non_database = DomainError::Storage(StorageError::Sqlx(sqlx::Error::RowNotFound));
        assert_eq!(non_database.code(), "internal_error");
    }
}
