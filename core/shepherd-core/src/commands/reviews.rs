use crate::error::DomainError;
use crate::workflow::eligibility::{GateCode, GateReason};

// plan/04: human-policy and self-review violations are 403, never a
// retryable eligibility gate.
pub(super) fn review_gate_error(reasons: &[GateReason]) -> DomainError {
    if reasons
        .iter()
        .any(|reason| reason.code == GateCode::ProducerCannotReview)
    {
        return DomainError::ProducerCannotReview;
    }
    if let Some(reason) = reasons
        .iter()
        .find(|reason| reason.code == GateCode::ReviewerPolicy)
    {
        return DomainError::ReviewerPolicy(reason.message.clone());
    }
    if reasons.is_empty() {
        return DomainError::NotEligible("no pending submission awaits review".into());
    }
    let joined: Vec<&str> = reasons.iter().map(|reason| reason.code.as_str()).collect();
    DomainError::NotEligible(joined.join(", "))
}
