//! Task lifecycle state machine.
//!
//! Pure functions — no IO, no storage access. The transition table is the
//! single source of truth for which status changes are legal.

use crate::error::Error;
use crate::model::TaskStatus;

/// What triggers a status transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// Human approves a proposed task.
    Approve,
    /// All `depends_on` predecessors are `done` (automatic).
    AutoReady,
    /// A caller claims the task.
    Claim,
    /// Claimant reports a successful session.
    SessionSuccess {
        /// Whether the project's review gate is enabled.
        review_gate: bool,
    },
    /// Human approves work in review.
    HumanApproval,
    /// Human rejects work in review — task returns to ready.
    HumanRejection,
    /// Human or agent blocks a task with a reason.
    Block,
    /// Unblock a previously blocked task (restores prior status).
    Unblock {
        /// The status the task was in before being blocked.
        blocked_from: TaskStatus,
    },
    /// Human cancels a task (terminal).
    Cancel,
    /// A new dependency was added and is not `done` — demote to approved.
    DependencyAdded,
}

/// Apply a transition: returns the new status, or an error if the transition
/// is illegal.
pub fn transition(current: TaskStatus, trigger: &Trigger) -> Result<TaskStatus, Error> {
    use TaskStatus::*;
    use Trigger::*;

    let next = match (current, trigger) {
        // Happy path
        (Proposed, Approve) => Approved,
        (Approved, AutoReady) => Ready,
        (Ready, Claim) => InProgress,
        (InProgress, SessionSuccess { review_gate: true }) => InReview,
        (InProgress, SessionSuccess { review_gate: false }) => Done,
        (InReview, HumanApproval) => Done,
        (InReview, HumanRejection) => Ready,

        // Ready demotion: new dependency not done
        (Ready, DependencyAdded) => Approved,

        // Block: any non-terminal status
        (s, Block) if !is_terminal(s) && s != Blocked => Blocked,

        // Unblock: restore previous status
        (Blocked, Unblock { blocked_from }) => *blocked_from,

        // Cancel: any non-terminal status
        (s, Cancel) if !is_terminal(s) => Cancelled,

        _ => {
            return Err(Error::InvalidTransition {
                from: current,
                trigger: format!("{trigger:?}"),
                detail: format!("cannot apply {trigger:?} to task in {current} status"),
            });
        }
    };
    Ok(next)
}

/// Whether a status is terminal (no outgoing transitions except cancel on
/// blocked).
pub fn is_terminal(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

/// All triggers that are valid from a given status.
pub fn allowed_triggers(status: TaskStatus) -> Vec<Trigger> {
    use TaskStatus::*;
    use Trigger::*;

    let mut triggers = Vec::new();
    match status {
        Proposed => {
            triggers.push(Approve);
            triggers.push(Block);
            triggers.push(Cancel);
        }
        Approved => {
            triggers.push(AutoReady);
            triggers.push(Block);
            triggers.push(Cancel);
        }
        Ready => {
            triggers.push(Claim);
            triggers.push(DependencyAdded);
            triggers.push(Block);
            triggers.push(Cancel);
        }
        InProgress => {
            triggers.push(SessionSuccess { review_gate: true });
            triggers.push(SessionSuccess { review_gate: false });
            triggers.push(Block);
            triggers.push(Cancel);
        }
        InReview => {
            triggers.push(HumanApproval);
            triggers.push(HumanRejection);
            triggers.push(Block);
            triggers.push(Cancel);
        }
        Blocked => {
            // Unblock requires knowing the previous status; we use a
            // placeholder here since the actual value comes from the DB.
            triggers.push(Unblock {
                blocked_from: Proposed,
            });
            triggers.push(Cancel);
        }
        Done | Cancelled => { /* terminal — no outgoing triggers */ }
    }
    triggers
}

#[cfg(test)]
mod tests {
    use super::*;
    use TaskStatus::*;
    use Trigger::*;

    #[test]
    fn happy_path_with_review_gate() {
        assert_eq!(transition(Proposed, &Approve).unwrap(), Approved);
        assert_eq!(transition(Approved, &AutoReady).unwrap(), Ready);
        assert_eq!(transition(Ready, &Claim).unwrap(), InProgress);
        assert_eq!(
            transition(InProgress, &SessionSuccess { review_gate: true }).unwrap(),
            InReview
        );
        assert_eq!(transition(InReview, &HumanApproval).unwrap(), Done);
    }

    #[test]
    fn happy_path_without_review_gate() {
        assert_eq!(
            transition(InProgress, &SessionSuccess { review_gate: false }).unwrap(),
            Done
        );
    }

    #[test]
    fn human_rejection_returns_to_ready() {
        assert_eq!(transition(InReview, &HumanRejection).unwrap(), Ready);
    }

    #[test]
    fn dependency_added_demotes_ready_to_approved() {
        assert_eq!(transition(Ready, &DependencyAdded).unwrap(), Approved);
    }

    #[test]
    fn block_from_any_non_terminal() {
        for status in [Proposed, Approved, Ready, InProgress, InReview] {
            assert_eq!(
                transition(status, &Block).unwrap(),
                Blocked,
                "block from {status}"
            );
        }
    }

    #[test]
    fn block_from_terminal_fails() {
        assert!(transition(Done, &Block).is_err());
        assert!(transition(Cancelled, &Block).is_err());
    }

    #[test]
    fn block_from_blocked_fails() {
        assert!(transition(Blocked, &Block).is_err());
    }

    #[test]
    fn unblock_restores_previous_status() {
        for original in [Proposed, Approved, Ready, InProgress, InReview] {
            assert_eq!(
                transition(
                    Blocked,
                    &Unblock {
                        blocked_from: original
                    }
                )
                .unwrap(),
                original,
                "unblock should restore {original}"
            );
        }
    }

    #[test]
    fn unblock_from_non_blocked_fails() {
        assert!(
            transition(
                Ready,
                &Unblock {
                    blocked_from: Ready
                }
            )
            .is_err()
        );
    }

    #[test]
    fn cancel_from_any_non_terminal() {
        for status in [Proposed, Approved, Ready, InProgress, InReview, Blocked] {
            assert_eq!(
                transition(status, &Cancel).unwrap(),
                Cancelled,
                "cancel from {status}"
            );
        }
    }

    #[test]
    fn cancel_from_terminal_fails() {
        assert!(transition(Done, &Cancel).is_err());
        assert!(transition(Cancelled, &Cancel).is_err());
    }

    #[test]
    fn invalid_transitions_return_error() {
        // Cannot approve an already-approved task
        assert!(transition(Approved, &Approve).is_err());
        // Cannot claim a proposed task
        assert!(transition(Proposed, &Claim).is_err());
        // Cannot auto-ready a proposed task
        assert!(transition(Proposed, &AutoReady).is_err());
        // Cannot report session on a ready task
        assert!(transition(Ready, &SessionSuccess { review_gate: true }).is_err());
    }

    #[test]
    fn terminal_states_are_correct() {
        assert!(is_terminal(Done));
        assert!(is_terminal(Cancelled));
        assert!(!is_terminal(Proposed));
        assert!(!is_terminal(Blocked));
    }

    #[test]
    fn allowed_triggers_are_valid() {
        for status in [
            Proposed, Approved, Ready, InProgress, InReview, Blocked, Done, Cancelled,
        ] {
            for trigger in allowed_triggers(status) {
                assert!(
                    transition(status, &trigger).is_ok(),
                    "allowed trigger {trigger:?} should be valid from {status}"
                );
            }
        }
    }
}
