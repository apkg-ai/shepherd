use shepherd_core::error::DomainError;
use shepherd_core::model as core;
use shepherd_core::queries::{ContextBundle, WorkPhase};

use crate::generated::server::api::ListWorkPhase;
use crate::generated::types as wire;

use super::{content, hierarchy};

pub(crate) fn claim_phase(phase: core::ClaimPhase) -> wire::ClaimPhase {
    match phase {
        core::ClaimPhase::Plan => wire::ClaimPhase::Plan,
        core::ClaimPhase::Execute => wire::ClaimPhase::Execute,
        core::ClaimPhase::Review => wire::ClaimPhase::Review,
    }
}

pub(crate) fn claim(claim: core::Claim) -> wire::Claim {
    wire::Claim {
        id: claim.id,
        task_id: claim.task_id.as_uuid(),
        actor_id: claim.actor_id.as_uuid(),
        phase: claim_phase(claim.phase),
        submission_ids: claim
            .submission_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        acquired_at: claim.acquired_at,
        expires_at: claim.expires_at,
        status: match claim.status {
            core::ClaimStatus::Active => wire::ClaimStatus::Active,
            core::ClaimStatus::Released => wire::ClaimStatus::Released,
            core::ClaimStatus::Expired => wire::ClaimStatus::Expired,
            core::ClaimStatus::Revoked => wire::ClaimStatus::Revoked,
            core::ClaimStatus::Reported => wire::ClaimStatus::Reported,
        },
        task_revision: claim.task_revision as i32,
        plan_revision_ids: claim
            .plan_revision_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

pub(crate) fn core_claim_input(input: wire::ClaimInput) -> core::ClaimInput {
    core::ClaimInput {
        phase: match input.phase {
            wire::ClaimInputPhase::Plan => core::ClaimPhase::Plan,
            wire::ClaimInputPhase::Execute => core::ClaimPhase::Execute,
            wire::ClaimInputPhase::Review => core::ClaimPhase::Review,
        },
        submission_id: input.submission_id,
        ttl_seconds: input.ttl_seconds.map(i64::from),
    }
}

pub(crate) fn core_renew_input(input: wire::RenewInput) -> core::RenewInput {
    core::RenewInput {
        ttl_seconds: input.ttl_seconds.map(i64::from),
    }
}

pub(crate) fn core_report_input(
    input: wire::ReportInput,
) -> Result<core::ReportInput, DomainError> {
    let document_revision_ids = input
        .document_revision_ids
        .iter()
        .map(|id| {
            id.parse().map_err(|_| DomainError::Validation {
                field: "document_revision_ids",
                message: "must contain UUIDs".into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(core::ReportInput {
        outcome: match input.outcome {
            wire::ReportInputOutcome::Succeeded => core::SessionOutcome::Succeeded,
            wire::ReportInputOutcome::Failed => core::SessionOutcome::Failed,
        },
        summary: input.summary,
        failure_reason: input.failure_reason.unwrap_or_default(),
        document_revision_ids,
        links: serde_json::json!(input.links),
    })
}

pub(crate) fn core_work_phase(phase: ListWorkPhase) -> WorkPhase {
    match phase {
        ListWorkPhase::Plan => WorkPhase::Plan,
        ListWorkPhase::Execute => WorkPhase::Execute,
        ListWorkPhase::Review => WorkPhase::Review,
    }
}

/// The bundle's task and epic arrive as pre-built wire bodies (the caller's
/// eligibility views); everything else converts from the snapshot.
pub(crate) fn context_bundle(
    bundle: ContextBundle,
    task: wire::Task,
    epic: wire::Epic,
) -> wire::ContextBundle {
    wire::ContextBundle {
        task,
        epic,
        goal: hierarchy::goal(bundle.goal),
        project: hierarchy::project(bundle.project),
        selected_plan: bundle
            .selected_plan
            .into_iter()
            .map(content::document_revision)
            .collect(),
        documents: bundle
            .documents
            .into_iter()
            .map(content::document_revision)
            .collect(),
        submissions: bundle
            .submissions
            .into_iter()
            .map(content::submission)
            .collect(),
        reviews: bundle.reviews.into_iter().map(content::review).collect(),
        sessions: bundle.sessions.into_iter().map(content::session).collect(),
        dependencies: bundle
            .dependencies
            .into_iter()
            .map(hierarchy::dependency)
            .collect(),
        sibling_claims: bundle.sibling_claims.into_iter().map(claim).collect(),
        truncated: bundle.truncated,
        continuation_paths: bundle.continuation_paths,
    }
}
