use shepherd_core::model as core;

use crate::generated::types as wire;

pub(crate) fn document_revision(revision: core::DocumentRevision) -> wire::DocumentRevision {
    wire::DocumentRevision {
        id: revision.id.as_uuid(),
        document_id: revision.document_id.as_uuid(),
        number: revision.number as i32,
        body: revision.body,
        links: revision.links,
        actor_id: revision.actor_id.as_uuid(),
        created_at: revision.created_at,
    }
}

pub(crate) fn session(session: core::Session) -> wire::Session {
    wire::Session {
        id: session.id,
        task_id: session.task_id.as_uuid(),
        claim_id: session.claim_id,
        actor_id: session.actor_id.as_uuid(),
        phase: match session.phase {
            core::ClaimPhase::Plan => wire::SessionPhase::Plan,
            core::ClaimPhase::Execute => wire::SessionPhase::Execute,
            // session_from_row rejects anything but plan/execute as corrupt
            // (503), so this arm cannot be reached from stored data.
            core::ClaimPhase::Review => unreachable!("review claims never record sessions"),
        },
        started_at: session.started_at,
        ended_at: session.ended_at,
        outcome: match session.outcome {
            core::SessionOutcome::Succeeded => wire::SessionOutcome::Succeeded,
            core::SessionOutcome::Failed => wire::SessionOutcome::Failed,
        },
        summary: session.summary,
        failure_reason: session.failure_reason,
        document_revision_ids: session
            .document_revision_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        // Stored as a validated JSON array of link strings (plan/03).
        links: session
            .links
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub(crate) fn submission(submission: core::Submission) -> wire::Submission {
    wire::Submission {
        id: submission.id,
        revision: submission.revision.value() as i32,
        created_at: submission.created_at,
        updated_at: submission.updated_at,
        task_id: submission.task_id.as_uuid(),
        kind: match submission.kind {
            core::SubmissionKind::Plan => wire::SubmissionKind::Plan,
            core::SubmissionKind::Work => wire::SubmissionKind::Work,
        },
        producer_id: submission.producer_id.as_uuid(),
        document_revision_ids: submission
            .document_revision_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        session_id: submission.session_id,
        policy: super::hierarchy::review_policy(submission.policy),
        status: match submission.status {
            core::SubmissionStatus::Pending => wire::SubmissionStatus::Pending,
            core::SubmissionStatus::Accepted => wire::SubmissionStatus::Accepted,
            core::SubmissionStatus::Rejected => wire::SubmissionStatus::Rejected,
            core::SubmissionStatus::Withdrawn => wire::SubmissionStatus::Withdrawn,
        },
        created_context_revision: submission.created_context_revision as i32,
    }
}

pub(crate) fn review(review: core::Review) -> wire::Review {
    wire::Review {
        id: review.id,
        submission_id: review.submission_id,
        actor_id: review.actor_id.as_uuid(),
        decision: match review.decision {
            core::ReviewDecision::Approve => wire::ReviewDecision::Approve,
            core::ReviewDecision::Reject => wire::ReviewDecision::Reject,
        },
        reason: review.reason,
        created_at: review.created_at,
    }
}

pub(crate) fn document_kind(kind: core::DocumentKind) -> wire::DocumentKind {
    match kind {
        core::DocumentKind::Brief => wire::DocumentKind::Brief,
        core::DocumentKind::Plan => wire::DocumentKind::Plan,
        core::DocumentKind::Finding => wire::DocumentKind::Finding,
        core::DocumentKind::Handoff => wire::DocumentKind::Handoff,
        core::DocumentKind::Note => wire::DocumentKind::Note,
        core::DocumentKind::Decision => wire::DocumentKind::Decision,
    }
}

pub(crate) fn owner_kind(kind: core::DocumentOwnerKind) -> wire::DocumentOwnerKind {
    match kind {
        core::DocumentOwnerKind::Project => wire::DocumentOwnerKind::Project,
        core::DocumentOwnerKind::Epic => wire::DocumentOwnerKind::Epic,
        core::DocumentOwnerKind::Task => wire::DocumentOwnerKind::Task,
    }
}

pub(crate) fn document(document: core::Document) -> wire::Document {
    wire::Document {
        id: document.id.as_uuid(),
        revision: document.revision.value() as i32,
        created_at: document.created_at,
        updated_at: document.updated_at,
        project_id: document.project_id.as_uuid(),
        owner_kind: owner_kind(document.owner_kind),
        owner_id: document.owner_id,
        kind: document_kind(document.kind),
        title: document.title,
        latest_revision_id: document.latest_revision_id.as_uuid(),
    }
}

pub(crate) fn core_document_create(input: wire::DocumentCreate) -> core::DocumentCreate {
    core::DocumentCreate {
        owner: core::DocumentOwner::from_parts(
            match input.owner_kind {
                wire::DocumentCreateOwnerKind::Project => core::DocumentOwnerKind::Project,
                wire::DocumentCreateOwnerKind::Epic => core::DocumentOwnerKind::Epic,
                wire::DocumentCreateOwnerKind::Task => core::DocumentOwnerKind::Task,
            },
            input.owner_id,
        ),
        kind: match input.kind {
            wire::DocumentCreateKind::Brief => core::DocumentKind::Brief,
            wire::DocumentCreateKind::Plan => core::DocumentKind::Plan,
            wire::DocumentCreateKind::Finding => core::DocumentKind::Finding,
            wire::DocumentCreateKind::Handoff => core::DocumentKind::Handoff,
            wire::DocumentCreateKind::Note => core::DocumentKind::Note,
            wire::DocumentCreateKind::Decision => core::DocumentKind::Decision,
        },
        title: input.title,
        body: input.body,
        links: input.links,
    }
}

pub(crate) fn core_revision_create(input: wire::RevisionCreate) -> core::RevisionCreate {
    core::RevisionCreate {
        body: input.body,
        links: input.links,
    }
}

pub(crate) fn core_review_input(input: wire::ReviewInput) -> core::ReviewInput {
    core::ReviewInput {
        decision: match input.decision {
            wire::ReviewInputDecision::Approve => core::ReviewDecision::Approve,
            wire::ReviewInputDecision::Reject => core::ReviewDecision::Reject,
        },
        reason: input.reason,
        claim_id: input.claim_id,
    }
}
