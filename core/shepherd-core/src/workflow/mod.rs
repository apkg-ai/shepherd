pub mod eligibility;
pub mod policy;

use chrono::{DateTime, Utc};
use sqlx::SqliteConnection;

use crate::commands::PendingEvent;
use crate::error::DomainError;
use crate::model::ProjectId;

/// Recompute epic completion and downstream cascades.
/// Body arrives when claims/reports can trigger state changes.
#[expect(dead_code)]
pub(crate) async fn recompute_affected(
    _tx: &mut SqliteConnection,
    _project: &ProjectId,
    _events: &mut Vec<PendingEvent>,
    _now: DateTime<Utc>,
) -> Result<(), DomainError> {
    Ok(())
}
