use crate::error::DomainError;
use crate::model::{
    ActorKind, EpicStatus, ProjectSettings, ReviewPolicy, TaskCreate, TaskPhase, TaskStatus,
};

#[derive(Debug)]
pub struct ResolvedPolicy {
    pub planning_required: bool,
    pub plan_review: ReviewPolicy,
    pub work_review: ReviewPolicy,
}

fn review_rank(policy: ReviewPolicy) -> u8 {
    match policy {
        ReviewPolicy::Human => 2,
        ReviewPolicy::Agent => 1,
        ReviewPolicy::None => 0,
    }
}

fn enforce_agent_floor(
    settings: &ProjectSettings,
    actor_kind: ActorKind,
    planning_required: bool,
    plan_review: ReviewPolicy,
    work_review: ReviewPolicy,
) -> Result<(), DomainError> {
    if actor_kind != ActorKind::Agent {
        return Ok(());
    }
    if !planning_required && settings.planning_required {
        return Err(DomainError::Forbidden(
            "agent cannot lower planning_required".into(),
        ));
    }
    if review_rank(plan_review) < review_rank(settings.plan_review) {
        return Err(DomainError::Forbidden(
            "agent cannot lower plan_review policy".into(),
        ));
    }
    if review_rank(work_review) < review_rank(settings.work_review) {
        return Err(DomainError::Forbidden(
            "agent cannot lower work_review policy".into(),
        ));
    }
    Ok(())
}

/// Resolve task policy from project defaults and creator input.
/// Agents cannot lower any field below the project default.
pub fn resolve_task_policy(
    settings: &ProjectSettings,
    actor_kind: ActorKind,
    input: &TaskCreate,
) -> Result<ResolvedPolicy, DomainError> {
    let planning_required = input
        .planning_required
        .unwrap_or(settings.planning_required);
    let plan_review = input.plan_review.unwrap_or(settings.plan_review);
    let work_review = input.work_review.unwrap_or(settings.work_review);
    enforce_agent_floor(
        settings,
        actor_kind,
        planning_required,
        plan_review,
        work_review,
    )?;
    Ok(ResolvedPolicy {
        planning_required,
        plan_review,
        work_review,
    })
}

/// Validate policy changes in an update patch. Same agent-cannot-lower rule.
pub fn validate_policy_update(
    settings: &ProjectSettings,
    actor_kind: ActorKind,
    planning_required: bool,
    plan_review: ReviewPolicy,
    work_review: ReviewPolicy,
) -> Result<(), DomainError> {
    enforce_agent_floor(
        settings,
        actor_kind,
        planning_required,
        plan_review,
        work_review,
    )
}

pub fn initial_phase(planning_required: bool) -> TaskPhase {
    if planning_required {
        TaskPhase::Planning
    } else {
        TaskPhase::Execution
    }
}

pub fn initial_status(actor_kind: ActorKind, proposal_gate: bool) -> TaskStatus {
    if actor_kind == ActorKind::Agent && proposal_gate {
        TaskStatus::Proposed
    } else {
        TaskStatus::Open
    }
}

pub fn initial_epic_status(actor_kind: ActorKind, proposal_gate: bool) -> EpicStatus {
    if actor_kind == ActorKind::Agent && proposal_gate {
        EpicStatus::Proposed
    } else {
        EpicStatus::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_settings() -> ProjectSettings {
        ProjectSettings::default()
    }

    fn permissive_settings() -> ProjectSettings {
        ProjectSettings {
            proposal_gate: false,
            planning_required: false,
            plan_review: ReviewPolicy::None,
            work_review: ReviewPolicy::None,
        }
    }

    fn strict_settings() -> ProjectSettings {
        ProjectSettings {
            proposal_gate: true,
            planning_required: true,
            plan_review: ReviewPolicy::Human,
            work_review: ReviewPolicy::Human,
        }
    }

    fn input(
        planning: Option<bool>,
        plan: Option<ReviewPolicy>,
        work: Option<ReviewPolicy>,
    ) -> TaskCreate {
        TaskCreate {
            title: "T".into(),
            type_key: "code".into(),
            planning_required: planning,
            plan_review: plan,
            work_review: work,
            ..Default::default()
        }
    }

    #[test]
    fn human_defaults_from_project_settings() {
        let resolved = resolve_task_policy(
            &default_settings(),
            ActorKind::Human,
            &input(None, None, None),
        )
        .unwrap();
        assert!(!resolved.planning_required);
        assert_eq!(resolved.plan_review, ReviewPolicy::Human);
        assert_eq!(resolved.work_review, ReviewPolicy::Human);
    }

    #[test]
    fn human_can_lower_policy() {
        let resolved = resolve_task_policy(
            &strict_settings(),
            ActorKind::Human,
            &input(
                Some(false),
                Some(ReviewPolicy::None),
                Some(ReviewPolicy::None),
            ),
        )
        .unwrap();
        assert!(!resolved.planning_required);
        assert_eq!(resolved.plan_review, ReviewPolicy::None);
    }

    #[test]
    fn agent_cannot_lower_planning_required() {
        let err = resolve_task_policy(
            &strict_settings(),
            ActorKind::Agent,
            &input(Some(false), None, None),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn agent_cannot_lower_review_policies() {
        let err = resolve_task_policy(
            &strict_settings(),
            ActorKind::Agent,
            &input(None, Some(ReviewPolicy::None), None),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        let err = resolve_task_policy(
            &strict_settings(),
            ActorKind::Agent,
            &input(None, None, Some(ReviewPolicy::Agent)),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn agent_can_raise_policy() {
        let resolved = resolve_task_policy(
            &permissive_settings(),
            ActorKind::Agent,
            &input(
                Some(true),
                Some(ReviewPolicy::Human),
                Some(ReviewPolicy::Human),
            ),
        )
        .unwrap();
        assert!(resolved.planning_required);
        assert_eq!(resolved.plan_review, ReviewPolicy::Human);
    }

    #[test]
    fn initial_phase_depends_on_planning_required() {
        assert_eq!(initial_phase(true), TaskPhase::Planning);
        assert_eq!(initial_phase(false), TaskPhase::Execution);
    }

    #[test]
    fn initial_status_depends_on_actor_and_gate() {
        assert_eq!(initial_status(ActorKind::Human, true), TaskStatus::Open);
        assert_eq!(initial_status(ActorKind::Human, false), TaskStatus::Open);
        assert_eq!(initial_status(ActorKind::Agent, false), TaskStatus::Open);
        assert_eq!(initial_status(ActorKind::Agent, true), TaskStatus::Proposed);
    }

    #[test]
    fn initial_epic_status_mirrors_task_logic() {
        assert_eq!(
            initial_epic_status(ActorKind::Agent, true),
            EpicStatus::Proposed
        );
        assert_eq!(
            initial_epic_status(ActorKind::Human, true),
            EpicStatus::Open
        );
    }

    #[test]
    fn review_ordering_human_above_agent_above_none() {
        assert!(review_rank(ReviewPolicy::Human) > review_rank(ReviewPolicy::Agent));
        assert!(review_rank(ReviewPolicy::Agent) > review_rank(ReviewPolicy::None));
    }
}
