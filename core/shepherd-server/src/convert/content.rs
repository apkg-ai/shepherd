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
            // sessions.phase is CHECK-constrained to plan/execute; review
            // claims record Review rows, never sessions.
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
