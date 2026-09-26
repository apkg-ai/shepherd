//! Pure eligibility evaluation (plan/04 FLOW-02): preloaded snapshots in,
//! verdict out. Loaders are the only storage-aware code; evaluators never
//! touch the database, so commands and queries share one implementation.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::{
    Actor, ActorId, ActorKind, Counts, Epic, EpicId, EpicStatus, GoalId, ProjectId, ReviewPolicy,
    Task, TaskId, TaskPhase, TaskStatus,
};
use crate::queries::hierarchy::{
    CountScope, EPIC_COLUMNS, TASK_COLUMNS, epic_from_row, parse_review_policy, project_archived,
    task_counts_by, task_from_row,
};
use crate::storage::StorageError;
use crate::storage::rows::{format_ts, parse_flag, parse_ts, parse_uuid};

/// Gate codes in exact contract order (openapi GateReason); reasons are always
/// emitted in this order for determinism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateCode {
    ProposalRequired,
    EpicProposalRequired,
    ExplicitBlock,
    EpicBlock,
    EpicPrerequisite,
    TaskPrerequisite,
    PlanRequired,
    PlanReviewRequired,
    WorkReviewRequired,
    Claimed,
    Terminal,
    Archived,
    WrongPhase,
    ReviewerPolicy,
    ProducerCannotReview,
}

impl GateCode {
    pub fn as_str(self) -> &'static str {
        match self {
            GateCode::ProposalRequired => "proposal_required",
            GateCode::EpicProposalRequired => "epic_proposal_required",
            GateCode::ExplicitBlock => "explicit_block",
            GateCode::EpicBlock => "epic_block",
            GateCode::EpicPrerequisite => "epic_prerequisite",
            GateCode::TaskPrerequisite => "task_prerequisite",
            GateCode::PlanRequired => "plan_required",
            GateCode::PlanReviewRequired => "plan_review_required",
            GateCode::WorkReviewRequired => "work_review_required",
            GateCode::Claimed => "claimed",
            GateCode::Terminal => "terminal",
            GateCode::Archived => "archived",
            GateCode::WrongPhase => "wrong_phase",
            GateCode::ReviewerPolicy => "reviewer_policy",
            GateCode::ProducerCannotReview => "producer_cannot_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateReason {
    pub code: GateCode,
    pub resource_id: Uuid,
    pub message: String,
}

/// Contract bound on Eligibility.reasons (openapi maxItems).
pub const MAX_REASONS: usize = 1_000;
/// Slots reserved for the bounded gate codes emitted around the prerequisite
/// reasons: every GateCode variant except the two prerequisite codes.
const PREREQ_REASON_BUDGET: usize = MAX_REASONS - 13;

impl GateReason {
    fn new(code: GateCode, resource_id: Uuid, message: &str) -> Self {
        Self {
            code,
            resource_id,
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eligibility {
    pub can_plan: bool,
    pub can_execute: bool,
    pub can_review: bool,
    pub reasons: Vec<GateReason>,
    pub allowed_actions: Vec<&'static str>,
}

/// Raw `status='active'` claim row; expiry is decided by the evaluator against
/// its `now` (plan/04: GET computes expired claims as inactive before sweep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimSnapshot {
    pub id: Uuid,
    pub actor_id: ActorId,
    /// Stored value ('plan'|'execute'|'review'); the claim model lands in step 009.
    pub phase: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionSnapshot {
    pub id: Uuid,
    /// Stored value ('plan'|'work'); the submission model lands in step 011.
    pub kind: String,
    pub producer_id: ActorId,
    pub policy: ReviewPolicy,
}

#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    pub task: Task,
    pub epic_status: EpicStatus,
    pub epic_blocked: bool,
    pub epic_archived: bool,
    pub goal_id: GoalId,
    pub goal_archived: bool,
    pub project_archived: bool,
    pub active_claim: Option<ClaimSnapshot>,
    pub pending_submission: Option<SubmissionSnapshot>,
    pub selected_plan_revision_id: Option<Uuid>,
    /// document_revision_ids of the accepted plan submission, if any.
    pub accepted_plan_revision_ids: Vec<Uuid>,
    /// Sorted by prerequisite id for deterministic reason output.
    pub task_prereqs: Vec<(TaskId, TaskStatus)>,
    /// Prerequisites of the owning epic, sorted by prerequisite id.
    pub epic_prereqs: Vec<(EpicId, EpicStatus)>,
}

#[derive(Debug, Clone)]
pub struct EpicSnapshot {
    pub epic: Epic,
    pub goal_archived: bool,
    pub project_archived: bool,
    pub task_counts: Counts,
    /// Any descendant task with an unexpired active claim or pending submission.
    pub active_descendant_work: bool,
    pub epic_prereqs: Vec<(EpicId, EpicStatus)>,
}

struct TaskFlags {
    can_plan: bool,
    can_execute: bool,
    can_review: bool,
}

const NO_FLAGS: TaskFlags = TaskFlags {
    can_plan: false,
    can_execute: false,
    can_review: false,
};

fn claim_is_active(claim: Option<&ClaimSnapshot>, now: DateTime<Utc>) -> bool {
    // Expiry is `expires_at <= now` (plan/04): a claim expiring exactly now is inactive.
    claim.is_some_and(|claim| claim.expires_at > now)
}

// First archived resource walking up from the entity, for the reason's resource id.
fn archived_task_resource(snapshot: &TaskSnapshot) -> Option<Uuid> {
    if snapshot.task.archived {
        Some(snapshot.task.id.as_uuid())
    } else if snapshot.epic_archived {
        Some(snapshot.task.epic_id.as_uuid())
    } else if snapshot.goal_archived {
        Some(snapshot.goal_id.as_uuid())
    } else if snapshot.project_archived {
        Some(snapshot.task.project_id.as_uuid())
    } else {
        None
    }
}

pub fn evaluate_task(snapshot: &TaskSnapshot, actor: &Actor, now: DateTime<Utc>) -> Eligibility {
    let task = &snapshot.task;

    // Archived/terminal exits early with a single reason (plan/04). Archival
    // first: valid archived records are always terminal (plan/03), so the
    // terminal gate would otherwise hide the archived one.
    if let Some(resource) = archived_task_resource(snapshot) {
        return Eligibility {
            can_plan: false,
            can_execute: false,
            can_review: false,
            reasons: vec![GateReason::new(
                GateCode::Archived,
                resource,
                "resource is archived",
            )],
            allowed_actions: allowed_task_actions(snapshot, actor, now, &NO_FLAGS),
        };
    }
    if task.status.is_terminal() || snapshot.epic_status.is_terminal() {
        let resource = if task.status.is_terminal() {
            task.id.as_uuid()
        } else {
            task.epic_id.as_uuid()
        };
        return Eligibility {
            can_plan: false,
            can_execute: false,
            can_review: false,
            reasons: vec![GateReason::new(
                GateCode::Terminal,
                resource,
                "resource is terminal",
            )],
            allowed_actions: allowed_task_actions(snapshot, actor, now, &NO_FLAGS),
        };
    }

    let accepted =
        task.status != TaskStatus::Proposed && snapshot.epic_status != EpicStatus::Proposed;
    let unblocked = task.block.is_none() && !snapshot.epic_blocked;
    let unclaimed = !claim_is_active(snapshot.active_claim.as_ref(), now);
    let unmet_epic: Vec<EpicId> = snapshot
        .epic_prereqs
        .iter()
        .filter(|(_, status)| *status != EpicStatus::Done)
        .map(|(id, _)| *id)
        .collect();
    let unmet_task: Vec<TaskId> = snapshot
        .task_prereqs
        .iter()
        .filter(|(_, status)| *status != TaskStatus::Done)
        .map(|(id, _)| *id)
        .collect();
    let deps_done = unmet_epic.is_empty() && unmet_task.is_empty();
    let plan_ok = !task.planning_required
        || snapshot
            .selected_plan_revision_id
            .is_some_and(|revision| snapshot.accepted_plan_revision_ids.contains(&revision));

    let submission = snapshot.pending_submission.as_ref();
    let review_kind = match task.phase {
        TaskPhase::PlanReview => Some("plan"),
        TaskPhase::WorkReview => Some("work"),
        _ => None,
    };
    let submission_matches_phase = matches!(
        (review_kind, submission),
        (Some(kind), Some(pending)) if pending.kind == kind
    );
    let policy_matches = submission.is_some_and(|pending| match pending.policy {
        ReviewPolicy::Human => actor.kind == ActorKind::Human,
        ReviewPolicy::Agent => actor.kind == ActorKind::Agent,
        ReviewPolicy::None => false,
    });
    let producer_conflict = submission.is_some_and(|pending| {
        pending.policy == ReviewPolicy::Agent && actor.id == pending.producer_id
    });

    let flags = TaskFlags {
        can_plan: accepted && unblocked && unclaimed && task.phase == TaskPhase::Planning,
        can_execute: accepted
            && unblocked
            && unclaimed
            && task.phase == TaskPhase::Execution
            && deps_done
            && plan_ok,
        can_review: accepted
            && unblocked
            && unclaimed
            && submission_matches_phase
            && policy_matches
            && !producer_conflict,
    };

    // Reasons in contract-enum order; prerequisite reasons sorted by resource id.
    let mut reasons = Vec::new();
    if task.status == TaskStatus::Proposed {
        reasons.push(GateReason::new(
            GateCode::ProposalRequired,
            task.id.as_uuid(),
            "task is proposed and awaits acceptance",
        ));
    }
    if snapshot.epic_status == EpicStatus::Proposed {
        reasons.push(GateReason::new(
            GateCode::EpicProposalRequired,
            task.epic_id.as_uuid(),
            "owning epic is proposed and awaits acceptance",
        ));
    }
    if task.block.is_some() {
        reasons.push(GateReason::new(
            GateCode::ExplicitBlock,
            task.id.as_uuid(),
            "task is explicitly blocked",
        ));
    }
    if snapshot.epic_blocked {
        reasons.push(GateReason::new(
            GateCode::EpicBlock,
            task.epic_id.as_uuid(),
            "owning epic is explicitly blocked",
        ));
    }
    // Prerequisite reasons are the only unbounded set — nothing caps stored
    // links — so the emitted list stays inside the contract bound while the
    // flags above still consider every prerequisite.
    let mut prereq_reasons = 0;
    for id in &unmet_epic {
        if prereq_reasons == PREREQ_REASON_BUDGET {
            break;
        }
        reasons.push(GateReason::new(
            GateCode::EpicPrerequisite,
            id.as_uuid(),
            "epic prerequisite is not done",
        ));
        prereq_reasons += 1;
    }
    for id in &unmet_task {
        if prereq_reasons == PREREQ_REASON_BUDGET {
            break;
        }
        reasons.push(GateReason::new(
            GateCode::TaskPrerequisite,
            id.as_uuid(),
            "task prerequisite is not done",
        ));
        prereq_reasons += 1;
    }
    if task.planning_required
        && !plan_ok
        && matches!(task.phase, TaskPhase::Planning | TaskPhase::Execution)
    {
        reasons.push(GateReason::new(
            GateCode::PlanRequired,
            task.id.as_uuid(),
            "an accepted plan is required before execution",
        ));
    }
    let review_resource = submission.map_or(task.id.as_uuid(), |pending| pending.id);
    if task.phase == TaskPhase::PlanReview {
        reasons.push(GateReason::new(
            GateCode::PlanReviewRequired,
            review_resource,
            "plan submission awaits review",
        ));
    }
    if task.phase == TaskPhase::WorkReview {
        reasons.push(GateReason::new(
            GateCode::WorkReviewRequired,
            review_resource,
            "work submission awaits review",
        ));
    }
    if let Some(claim) = snapshot.active_claim.as_ref()
        && claim_is_active(Some(claim), now)
    {
        reasons.push(GateReason::new(
            GateCode::Claimed,
            claim.id,
            "task has an active claim",
        ));
    }
    if let Some(pending) = submission {
        if !submission_matches_phase {
            reasons.push(GateReason::new(
                GateCode::WrongPhase,
                pending.id,
                "pending submission does not match the current phase",
            ));
        } else {
            if !policy_matches {
                reasons.push(GateReason::new(
                    GateCode::ReviewerPolicy,
                    pending.id,
                    "caller does not match the review policy",
                ));
            }
            if producer_conflict {
                reasons.push(GateReason::new(
                    GateCode::ProducerCannotReview,
                    pending.id,
                    "producer cannot review its own submission",
                ));
            }
        }
    }

    let allowed_actions = allowed_task_actions(snapshot, actor, now, &flags);
    Eligibility {
        can_plan: flags.can_plan,
        can_execute: flags.can_execute,
        can_review: flags.can_review,
        reasons,
        allowed_actions,
    }
}

// Eligibility ∩ capability matrix (plan/12); strings are REST operationIds and
// authoritative for buttons. Fixed emission order for determinism.
fn allowed_task_actions(
    snapshot: &TaskSnapshot,
    actor: &Actor,
    now: DateTime<Utc>,
    flags: &TaskFlags,
) -> Vec<&'static str> {
    if snapshot.task.archived
        || snapshot.epic_archived
        || snapshot.goal_archived
        || snapshot.project_archived
    {
        return Vec::new();
    }
    let task = &snapshot.task;
    let owner = actor.kind == ActorKind::Human;
    let terminal = task.status.is_terminal();
    let epic_live = !snapshot.epic_status.is_terminal();
    let claim = snapshot
        .active_claim
        .as_ref()
        .filter(|claim| claim_is_active(Some(claim), now));
    let unclaimed = claim.is_none();
    let pending = snapshot.pending_submission.is_some();

    let mut actions = Vec::new();
    if !terminal && epic_live && unclaimed && !pending {
        actions.push("updateTask");
    }
    if owner && task.status == TaskStatus::Proposed && epic_live {
        actions.push("acceptTask");
    }
    if !terminal && task.block.is_none() {
        actions.push("blockTask");
    }
    if owner && task.block.is_some() {
        actions.push("unblockTask");
    }
    if owner && !terminal {
        actions.push("cancelTask");
    }
    if owner && task.status == TaskStatus::Cancelled && task.waiver.is_none() && epic_live {
        actions.push("waiveTask");
    }
    if owner && terminal && unclaimed && !pending {
        actions.push("archiveTask");
    }
    // Agent review acquisition is a claim; human review is claimless (plan/04).
    if flags.can_plan || flags.can_execute || (flags.can_review && actor.kind == ActorKind::Agent) {
        actions.push("claimTask");
    }
    // Only the plan claimant may save/select while its claim is active; execute/
    // review claims and pending reviews block requirement changes (plan/04 FLOW-03).
    if task.planning_required
        && !terminal
        && epic_live
        && !pending
        && claim.is_none_or(|claim| claim.phase == "plan" && claim.actor_id == actor.id)
    {
        actions.push("selectTaskPlan");
    }
    if flags.can_review && owner {
        actions.push("reviewSubmission");
    }
    // Agents may only link accepted dependents (plan/03): a proposed epic
    // rejects them in create_dependency just like a proposed task does.
    if !terminal
        && epic_live
        && unclaimed
        && !pending
        && (owner
            || (task.status != TaskStatus::Proposed
                && snapshot.epic_status != EpicStatus::Proposed))
    {
        actions.push("createDependency");
    }
    actions
}

pub fn evaluate_epic(snapshot: &EpicSnapshot, actor: &Actor, _now: DateTime<Utc>) -> Eligibility {
    let epic = &snapshot.epic;
    let allowed_actions = allowed_epic_actions(snapshot, actor);

    if epic.archived || snapshot.goal_archived || snapshot.project_archived {
        let resource = if epic.archived {
            epic.id.as_uuid()
        } else if snapshot.goal_archived {
            epic.goal_id.as_uuid()
        } else {
            epic.project_id.as_uuid()
        };
        return Eligibility {
            can_plan: false,
            can_execute: false,
            can_review: false,
            reasons: vec![GateReason::new(
                GateCode::Archived,
                resource,
                "resource is archived",
            )],
            allowed_actions,
        };
    }
    if epic.status.is_terminal() {
        return Eligibility {
            can_plan: false,
            can_execute: false,
            can_review: false,
            reasons: vec![GateReason::new(
                GateCode::Terminal,
                epic.id.as_uuid(),
                "resource is terminal",
            )],
            allowed_actions,
        };
    }

    let accepted = epic.status != EpicStatus::Proposed;
    let unblocked = epic.block.is_none();
    let unmet: Vec<EpicId> = snapshot
        .epic_prereqs
        .iter()
        .filter(|(_, status)| *status != EpicStatus::Done)
        .map(|(id, _)| *id)
        .collect();

    let mut reasons = Vec::new();
    if epic.status == EpicStatus::Proposed {
        reasons.push(GateReason::new(
            GateCode::ProposalRequired,
            epic.id.as_uuid(),
            "epic is proposed and awaits acceptance",
        ));
    }
    if epic.block.is_some() {
        reasons.push(GateReason::new(
            GateCode::ExplicitBlock,
            epic.id.as_uuid(),
            "epic is explicitly blocked",
        ));
    }
    // Same contract-bound cap as evaluate_task; see the note there.
    for id in unmet.iter().take(PREREQ_REASON_BUDGET) {
        reasons.push(GateReason::new(
            GateCode::EpicPrerequisite,
            id.as_uuid(),
            "epic prerequisite is not done",
        ));
    }

    Eligibility {
        can_plan: accepted && unblocked,
        can_execute: accepted && unblocked && unmet.is_empty(),
        can_review: false,
        reasons,
        allowed_actions,
    }
}

fn allowed_epic_actions(snapshot: &EpicSnapshot, actor: &Actor) -> Vec<&'static str> {
    if snapshot.epic.archived || snapshot.goal_archived || snapshot.project_archived {
        return Vec::new();
    }
    let epic = &snapshot.epic;
    let owner = actor.kind == ActorKind::Human;
    let terminal = epic.status.is_terminal();
    let accepted = epic.status != EpicStatus::Proposed;
    let unblocked = epic.block.is_none();
    let deps_done = snapshot
        .epic_prereqs
        .iter()
        .all(|(_, status)| *status == EpicStatus::Done);
    let counts = snapshot.task_counts;

    let mut actions = Vec::new();
    if !terminal {
        actions.push("updateEpic");
    }
    if owner && epic.status == EpicStatus::Proposed {
        actions.push("acceptEpic");
    }
    if !terminal && unblocked {
        actions.push("blockEpic");
    }
    if owner && epic.block.is_some() {
        actions.push("unblockEpic");
    }
    if owner && !terminal {
        actions.push("cancelEpic");
    }
    // Explicit completion covers empty or all-waived epics (plan/04).
    if owner
        && !terminal
        && accepted
        && unblocked
        && deps_done
        && (counts.total == 0 || counts.waived == counts.total)
    {
        actions.push("completeEpic");
    }
    if owner && terminal && !snapshot.active_descendant_work {
        actions.push("archiveEpic");
    }
    if !terminal && !snapshot.active_descendant_work && (owner || accepted) {
        actions.push("createDependency");
    }
    actions
}

fn push_in_list<T: std::fmt::Display>(
    builder: &mut QueryBuilder<Sqlite>,
    values: impl Iterator<Item = T>,
) {
    let mut separated = builder.separated(", ");
    for value in values {
        separated.push_bind(value.to_string());
    }
}

fn parse_revision_ids(raw: &str) -> Result<Vec<Uuid>, DomainError> {
    let ids: Vec<String> = serde_json::from_str(raw).map_err(|err| {
        StorageError::Corrupt(format!("submissions.document_revision_ids: {err}"))
    })?;
    ids.iter()
        .map(|id| parse_uuid("submissions.document_revision_ids", id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(DomainError::from)
}

/// Batch snapshot loader: constant query count regardless of batch size
/// (plan/05: no per-node database queries).
pub(crate) async fn load_task_snapshots(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    ids: &[TaskId],
) -> Result<HashMap<TaskId, TaskSnapshot>, DomainError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {TASK_COLUMNS}, selected_plan_revision_id, accepted_plan_submission_id \
         FROM tasks WHERE project_id = "
    ));
    builder.push_bind(project.to_string()).push(" AND id IN (");
    push_in_list(&mut builder, ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    struct TaskRow {
        task: Task,
        selected_plan_revision_id: Option<Uuid>,
        accepted_plan_submission_id: Option<Uuid>,
    }
    let mut tasks = Vec::with_capacity(rows.len());
    for row in &rows {
        let selected: Option<String> = row.try_get("selected_plan_revision_id")?;
        let accepted: Option<String> = row.try_get("accepted_plan_submission_id")?;
        tasks.push(TaskRow {
            task: task_from_row(row)?,
            selected_plan_revision_id: selected
                .map(|id| parse_uuid("tasks.selected_plan_revision_id", &id))
                .transpose()?,
            accepted_plan_submission_id: accepted
                .map(|id| parse_uuid("tasks.accepted_plan_submission_id", &id))
                .transpose()?,
        });
    }

    let epic_ids: HashSet<EpicId> = tasks.iter().map(|row| row.task.epic_id).collect();
    struct EpicMeta {
        goal_id: Uuid,
        status: EpicStatus,
        archived: bool,
        blocked: bool,
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, goal_id, status, archived, block_actor_id FROM epics WHERE id IN (",
    );
    push_in_list(&mut builder, epic_ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut epics: HashMap<Uuid, EpicMeta> = HashMap::new();
    for row in &rows {
        let id: String = row.try_get("id")?;
        let goal_id: String = row.try_get("goal_id")?;
        let status: String = row.try_get("status")?;
        let block: Option<String> = row.try_get("block_actor_id")?;
        epics.insert(
            parse_uuid("epics.id", &id)?,
            EpicMeta {
                goal_id: parse_uuid("epics.goal_id", &goal_id)?,
                status: EpicStatus::parse(&status)
                    .ok_or_else(|| StorageError::Corrupt(format!("epics.status: {status:?}")))?,
                archived: parse_flag("epics.archived", row.try_get("archived")?)?,
                blocked: block.is_some(),
            },
        );
    }

    let goal_ids: HashSet<Uuid> = epics.values().map(|meta| meta.goal_id).collect();
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT id, archived FROM goals WHERE id IN (");
    push_in_list(&mut builder, goal_ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut goals: HashMap<Uuid, bool> = HashMap::new();
    for row in &rows {
        let id: String = row.try_get("id")?;
        goals.insert(
            parse_uuid("goals.id", &id)?,
            parse_flag("goals.archived", row.try_get("archived")?)?,
        );
    }

    let project_is_archived = project_archived(conn, project).await?;

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT task_id, id, actor_id, phase, expires_at FROM claims \
         WHERE status = 'active' AND task_id IN (",
    );
    push_in_list(&mut builder, ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    // one_active_claim_per_task (partial unique index) guarantees one row per key.
    let mut claims: HashMap<Uuid, ClaimSnapshot> = HashMap::new();
    for row in &rows {
        let task_id: String = row.try_get("task_id")?;
        let id: String = row.try_get("id")?;
        let actor_id: String = row.try_get("actor_id")?;
        let expires_at: String = row.try_get("expires_at")?;
        claims.insert(
            parse_uuid("claims.task_id", &task_id)?,
            ClaimSnapshot {
                id: parse_uuid("claims.id", &id)?,
                actor_id: ActorId::from_uuid(parse_uuid("claims.actor_id", &actor_id)?),
                phase: row.try_get("phase")?,
                expires_at: parse_ts("claims.expires_at", &expires_at)?,
            },
        );
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT task_id, id, kind, producer_id, policy FROM submissions \
         WHERE status = 'pending' AND task_id IN (",
    );
    push_in_list(&mut builder, ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    // one_pending_submission_per_task (partial unique index) guarantees one row per key.
    let mut submissions: HashMap<Uuid, SubmissionSnapshot> = HashMap::new();
    for row in &rows {
        let task_id: String = row.try_get("task_id")?;
        let id: String = row.try_get("id")?;
        let producer_id: String = row.try_get("producer_id")?;
        let policy: String = row.try_get("policy")?;
        submissions.insert(
            parse_uuid("submissions.task_id", &task_id)?,
            SubmissionSnapshot {
                id: parse_uuid("submissions.id", &id)?,
                kind: row.try_get("kind")?,
                producer_id: ActorId::from_uuid(parse_uuid(
                    "submissions.producer_id",
                    &producer_id,
                )?),
                policy: parse_review_policy("submissions.policy", &policy)?,
            },
        );
    }

    let accepted_ids: HashSet<Uuid> = tasks
        .iter()
        .filter_map(|row| row.accepted_plan_submission_id)
        .collect();
    let mut accepted_revisions: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    if !accepted_ids.is_empty() {
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, document_revision_ids FROM submissions WHERE id IN (",
        );
        push_in_list(&mut builder, accepted_ids.iter());
        builder.push(")");
        let rows = builder.build().fetch_all(&mut *conn).await?;
        for row in &rows {
            let id: String = row.try_get("id")?;
            let raw: String = row.try_get("document_revision_ids")?;
            accepted_revisions.insert(
                parse_uuid("submissions.id", &id)?,
                parse_revision_ids(&raw)?,
            );
        }
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT d.dependent_id, d.prerequisite_id, p.status FROM task_dependencies d \
         JOIN tasks p ON p.id = d.prerequisite_id WHERE d.dependent_id IN (",
    );
    push_in_list(&mut builder, ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut task_prereqs: HashMap<Uuid, Vec<(TaskId, TaskStatus)>> = HashMap::new();
    for row in &rows {
        let dependent: String = row.try_get("dependent_id")?;
        let prerequisite: String = row.try_get("prerequisite_id")?;
        let status: String = row.try_get("status")?;
        task_prereqs
            .entry(parse_uuid("task_dependencies.dependent_id", &dependent)?)
            .or_default()
            .push((
                TaskId::from_uuid(parse_uuid(
                    "task_dependencies.prerequisite_id",
                    &prerequisite,
                )?),
                TaskStatus::parse(&status)
                    .ok_or_else(|| StorageError::Corrupt(format!("tasks.status: {status:?}")))?,
            ));
    }

    let epic_prereqs = load_epic_prereqs(conn, epic_ids.iter().map(EpicId::as_uuid)).await?;

    let mut snapshots = HashMap::with_capacity(tasks.len());
    for row in tasks {
        let task = row.task;
        let epic = epics
            .get(&task.epic_id.as_uuid())
            .ok_or_else(|| StorageError::Corrupt(format!("tasks.epic_id: {}", task.epic_id)))?;
        let goal_archived = goals
            .get(&epic.goal_id)
            .copied()
            .ok_or_else(|| StorageError::Corrupt(format!("epics.goal_id: {}", epic.goal_id)))?;
        let accepted_plan_revision_ids = row
            .accepted_plan_submission_id
            .and_then(|id| accepted_revisions.get(&id).cloned())
            .unwrap_or_default();
        let mut own_task_prereqs = task_prereqs.remove(&task.id.as_uuid()).unwrap_or_default();
        own_task_prereqs.sort_by_key(|(id, _)| *id);
        let mut own_epic_prereqs = epic_prereqs
            .get(&task.epic_id.as_uuid())
            .cloned()
            .unwrap_or_default();
        own_epic_prereqs.sort_by_key(|(id, _)| *id);
        let id = task.id;
        snapshots.insert(
            id,
            TaskSnapshot {
                active_claim: claims.remove(&task.id.as_uuid()),
                pending_submission: submissions.remove(&task.id.as_uuid()),
                selected_plan_revision_id: row.selected_plan_revision_id,
                accepted_plan_revision_ids,
                task_prereqs: own_task_prereqs,
                epic_prereqs: own_epic_prereqs,
                epic_status: epic.status,
                epic_blocked: epic.blocked,
                epic_archived: epic.archived,
                goal_id: GoalId::from_uuid(epic.goal_id),
                goal_archived,
                project_archived: project_is_archived,
                task,
            },
        );
    }
    Ok(snapshots)
}

async fn load_epic_prereqs(
    conn: &mut SqliteConnection,
    dependents: impl Iterator<Item = Uuid>,
) -> Result<HashMap<Uuid, Vec<(EpicId, EpicStatus)>>, DomainError> {
    let dependents: Vec<Uuid> = dependents.collect();
    if dependents.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT d.dependent_id, d.prerequisite_id, e.status FROM epic_dependencies d \
         JOIN epics e ON e.id = d.prerequisite_id WHERE d.dependent_id IN (",
    );
    push_in_list(&mut builder, dependents.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut prereqs: HashMap<Uuid, Vec<(EpicId, EpicStatus)>> = HashMap::new();
    for row in &rows {
        let dependent: String = row.try_get("dependent_id")?;
        let prerequisite: String = row.try_get("prerequisite_id")?;
        let status: String = row.try_get("status")?;
        prereqs
            .entry(parse_uuid("epic_dependencies.dependent_id", &dependent)?)
            .or_default()
            .push((
                EpicId::from_uuid(parse_uuid(
                    "epic_dependencies.prerequisite_id",
                    &prerequisite,
                )?),
                EpicStatus::parse(&status)
                    .ok_or_else(|| StorageError::Corrupt(format!("epics.status: {status:?}")))?,
            ));
    }
    Ok(prereqs)
}

/// Batch epic snapshot loader. `now` bakes claim expiry into the descendant
/// aggregate only; task-level expiry stays raw for the evaluator.
pub(crate) async fn load_epic_snapshots(
    conn: &mut SqliteConnection,
    project: &ProjectId,
    ids: &[EpicId],
    now: DateTime<Utc>,
) -> Result<HashMap<EpicId, EpicSnapshot>, DomainError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {EPIC_COLUMNS} FROM epics WHERE project_id = "
    ));
    builder.push_bind(project.to_string()).push(" AND id IN (");
    push_in_list(&mut builder, ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let epics = rows
        .iter()
        .map(epic_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let goal_ids: HashSet<Uuid> = epics.iter().map(|epic| epic.goal_id.as_uuid()).collect();
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT id, archived FROM goals WHERE id IN (");
    push_in_list(&mut builder, goal_ids.iter());
    builder.push(")");
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut goals: HashMap<Uuid, bool> = HashMap::new();
    for row in &rows {
        let id: String = row.try_get("id")?;
        goals.insert(
            parse_uuid("goals.id", &id)?,
            parse_flag("goals.archived", row.try_get("archived")?)?,
        );
    }

    let project_is_archived = project_archived(conn, project).await?;
    let epic_prereqs = load_epic_prereqs(conn, ids.iter().map(EpicId::as_uuid)).await?;

    let uuids: Vec<Uuid> = ids.iter().map(EpicId::as_uuid).collect();
    let counts = task_counts_by(conn, CountScope::Epic, &uuids).await?;

    let mut builder =
        QueryBuilder::<Sqlite>::new("SELECT DISTINCT t.epic_id FROM tasks t WHERE t.epic_id IN (");
    push_in_list(&mut builder, ids.iter());
    builder
        .push(
            ") AND (EXISTS (SELECT 1 FROM claims c WHERE c.task_id = t.id \
             AND c.status = 'active' AND c.expires_at > ",
        )
        .push_bind(format_ts(&now))
        .push(
            ") OR EXISTS (SELECT 1 FROM submissions s WHERE s.task_id = t.id \
             AND s.status = 'pending'))",
        );
    let rows = builder.build().fetch_all(&mut *conn).await?;
    let mut busy: HashSet<Uuid> = HashSet::new();
    for row in &rows {
        let id: String = row.try_get("epic_id")?;
        busy.insert(parse_uuid("tasks.epic_id", &id)?);
    }

    let mut snapshots = HashMap::with_capacity(epics.len());
    for epic in epics {
        let goal_archived = goals
            .get(&epic.goal_id.as_uuid())
            .copied()
            .ok_or_else(|| StorageError::Corrupt(format!("epics.goal_id: {}", epic.goal_id)))?;
        let mut own_prereqs = epic_prereqs
            .get(&epic.id.as_uuid())
            .cloned()
            .unwrap_or_default();
        own_prereqs.sort_by_key(|(id, _)| *id);
        let id = epic.id;
        snapshots.insert(
            id,
            EpicSnapshot {
                goal_archived,
                project_archived: project_is_archived,
                task_counts: counts
                    .get(&epic.id.as_uuid())
                    .copied()
                    .unwrap_or(Counts::ZERO),
                active_descendant_work: busy.contains(&epic.id.as_uuid()),
                epic_prereqs: own_prereqs,
                epic,
            },
        );
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LifecycleRecord, Revision};

    fn ts(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    fn now() -> DateTime<Utc> {
        ts("2026-09-14T00:00:00Z")
    }

    fn person(kind: ActorKind) -> Actor {
        Actor {
            id: ActorId::generate(now()),
            kind,
            label: kind.as_str().to_string(),
            revoked: false,
            created_at: now(),
        }
    }

    fn task() -> Task {
        Task {
            id: TaskId::generate(now()),
            revision: Revision::INITIAL,
            created_at: now(),
            updated_at: now(),
            project_id: ProjectId::generate(now()),
            epic_id: EpicId::generate(now()),
            title: "T".to_string(),
            description: String::new(),
            type_key: "code".to_string(),
            status: TaskStatus::Open,
            phase: TaskPhase::Execution,
            planning_required: false,
            plan_review: ReviewPolicy::Human,
            work_review: ReviewPolicy::Human,
            archived: false,
            attempt_count: 0,
            block: None,
            waiver: None,
            archive: None,
            cancellation: None,
        }
    }

    fn snapshot(task: Task) -> TaskSnapshot {
        TaskSnapshot {
            task,
            epic_status: EpicStatus::Open,
            epic_blocked: false,
            epic_archived: false,
            goal_id: GoalId::generate(now()),
            goal_archived: false,
            project_archived: false,
            active_claim: None,
            pending_submission: None,
            selected_plan_revision_id: None,
            accepted_plan_revision_ids: Vec::new(),
            task_prereqs: Vec::new(),
            epic_prereqs: Vec::new(),
        }
    }

    fn block() -> LifecycleRecord {
        LifecycleRecord {
            actor_id: ActorId::generate(now()),
            reason: "hold".to_string(),
            created_at: now(),
        }
    }

    fn claim(expires_at: DateTime<Utc>) -> ClaimSnapshot {
        ClaimSnapshot {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            actor_id: ActorId::generate(now()),
            phase: "execute".to_string(),
            expires_at,
        }
    }

    fn submission(kind: &str, policy: ReviewPolicy, producer: ActorId) -> SubmissionSnapshot {
        SubmissionSnapshot {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            kind: kind.to_string(),
            producer_id: producer,
            policy,
        }
    }

    fn codes(eligibility: &Eligibility) -> Vec<&'static str> {
        eligibility
            .reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect()
    }

    #[test]
    fn open_execution_task_with_no_gates_is_executable() {
        let verdict = evaluate_task(&snapshot(task()), &person(ActorKind::Agent), now());
        assert!(!verdict.can_plan);
        assert!(verdict.can_execute);
        assert!(!verdict.can_review);
        assert!(verdict.reasons.is_empty());
        assert_eq!(
            verdict.allowed_actions,
            vec!["updateTask", "blockTask", "claimTask", "createDependency"]
        );
    }

    #[test]
    fn owner_actions_extend_the_agent_set() {
        let verdict = evaluate_task(&snapshot(task()), &person(ActorKind::Human), now());
        assert_eq!(
            verdict.allowed_actions,
            vec![
                "updateTask",
                "blockTask",
                "cancelTask",
                "claimTask",
                "createDependency"
            ]
        );
    }

    #[test]
    fn terminal_task_exits_early_with_a_single_reason() {
        let mut task = task();
        task.status = TaskStatus::Done;
        task.phase = TaskPhase::Complete;
        let expected = task.id.as_uuid();
        let verdict = evaluate_task(&snapshot(task), &person(ActorKind::Human), now());
        assert!(!verdict.can_plan && !verdict.can_execute && !verdict.can_review);
        assert_eq!(codes(&verdict), vec!["terminal"]);
        assert_eq!(verdict.reasons[0].resource_id, expected);
        assert_eq!(verdict.allowed_actions, vec!["archiveTask"]);
    }

    #[test]
    fn terminal_epic_exits_early_with_the_epic_resource() {
        let task = task();
        let expected = task.epic_id.as_uuid();
        let mut snapshot = snapshot(task);
        snapshot.epic_status = EpicStatus::Cancelled;
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["terminal"]);
        assert_eq!(verdict.reasons[0].resource_id, expected);
    }

    #[test]
    fn cancelled_task_offers_waive_and_archive_to_the_owner() {
        let mut task = task();
        task.status = TaskStatus::Cancelled;
        task.phase = TaskPhase::Complete;
        let verdict = evaluate_task(&snapshot(task), &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["terminal"]);
        assert_eq!(verdict.allowed_actions, vec!["waiveTask", "archiveTask"]);
    }

    #[test]
    fn archived_chain_exits_early_and_empties_actions() {
        for (task_archived, epic_archived, goal_archived, project_archived) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            let mut task = task();
            task.archived = task_archived;
            let mut snapshot = snapshot(task);
            snapshot.epic_archived = epic_archived;
            snapshot.goal_archived = goal_archived;
            snapshot.project_archived = project_archived;
            let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
            assert_eq!(codes(&verdict), vec!["archived"]);
            // The reason points at the archived resource itself.
            let expected = if task_archived {
                snapshot.task.id.as_uuid()
            } else if epic_archived {
                snapshot.task.epic_id.as_uuid()
            } else if goal_archived {
                snapshot.goal_id.as_uuid()
            } else {
                snapshot.task.project_id.as_uuid()
            };
            assert_eq!(verdict.reasons[0].resource_id, expected);
            assert!(verdict.allowed_actions.is_empty());
        }
    }

    #[test]
    fn archived_terminal_records_report_the_archived_gate() {
        // Valid archived records are always terminal (plan/03); the archived
        // gate must win so archived and live terminal work stay distinct.
        let mut task = task();
        task.status = TaskStatus::Done;
        task.phase = TaskPhase::Complete;
        task.archived = true;
        let snapshot = snapshot(task);
        let expected = snapshot.task.id.as_uuid();
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["archived"]);
        assert_eq!(verdict.reasons[0].resource_id, expected);
        assert!(verdict.allowed_actions.is_empty());

        let mut epic = epic();
        epic.status = EpicStatus::Done;
        epic.archived = true;
        let verdict = evaluate_epic(&epic_snapshot(epic), &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["archived"]);
    }

    #[test]
    fn gates_report_in_contract_order() {
        let mut task = task();
        task.status = TaskStatus::Proposed;
        task.block = Some(block());
        let mut snapshot = snapshot(task);
        snapshot.epic_status = EpicStatus::Proposed;
        snapshot.epic_blocked = true;
        snapshot.epic_prereqs = vec![(EpicId::generate(now()), EpicStatus::Open)];
        snapshot.task_prereqs = vec![(TaskId::generate(now()), TaskStatus::Active)];
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert_eq!(
            codes(&verdict),
            vec![
                "proposal_required",
                "epic_proposal_required",
                "explicit_block",
                "epic_block",
                "epic_prerequisite",
                "task_prerequisite",
            ]
        );
        assert!(!verdict.can_plan && !verdict.can_execute && !verdict.can_review);
    }

    #[test]
    fn prerequisite_reasons_carry_blocking_resource_ids() {
        let first = TaskId::generate(now());
        let second = TaskId::generate(ts("2026-09-15T00:00:00Z"));
        let mut snapshot = snapshot(task());
        snapshot.task_prereqs = vec![(first, TaskStatus::Open), (second, TaskStatus::Cancelled)];
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert_eq!(
            codes(&verdict),
            vec!["task_prerequisite", "task_prerequisite"]
        );
        assert_eq!(verdict.reasons[0].resource_id, first.as_uuid());
        assert_eq!(verdict.reasons[1].resource_id, second.as_uuid());
        assert!(!verdict.can_execute);
    }

    #[test]
    fn prerequisite_reasons_stay_within_the_contract_bound() {
        // Nothing caps stored links, so unmet prerequisites are capped at
        // emit time to keep Eligibility.reasons inside the openapi maxItems.
        let mut snapshot = snapshot(task());
        snapshot.task_prereqs = (0..MAX_REASONS as i64 + 200)
            .map(|i| {
                (
                    TaskId::generate(ts("2026-09-15T00:00:00Z") + chrono::Duration::seconds(i)),
                    TaskStatus::Open,
                )
            })
            .collect();
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(!verdict.can_execute);
        assert!(verdict.reasons.len() <= MAX_REASONS);
        assert_eq!(verdict.reasons.len(), PREREQ_REASON_BUDGET);
        assert!(
            verdict
                .reasons
                .iter()
                .all(|reason| reason.code == GateCode::TaskPrerequisite)
        );

        let mut epic_snapshot = epic_snapshot(epic());
        epic_snapshot.epic_prereqs = snapshot
            .task_prereqs
            .iter()
            .map(|(id, _)| (EpicId::from_uuid(id.as_uuid()), EpicStatus::Open))
            .collect();
        let verdict = evaluate_epic(&epic_snapshot, &person(ActorKind::Human), now());
        assert_eq!(verdict.reasons.len(), PREREQ_REASON_BUDGET);
    }

    #[test]
    fn cancelled_prerequisites_stay_unmet_and_done_satisfies() {
        let mut snapshot = snapshot(task());
        snapshot.task_prereqs = vec![(TaskId::generate(now()), TaskStatus::Cancelled)];
        assert!(!evaluate_task(&snapshot, &person(ActorKind::Human), now()).can_execute);
        snapshot.task_prereqs = vec![(TaskId::generate(now()), TaskStatus::Done)];
        assert!(evaluate_task(&snapshot, &person(ActorKind::Human), now()).can_execute);
    }

    #[test]
    fn early_planning_ignores_dependency_waits_but_not_blocks() {
        let mut task = task();
        task.phase = TaskPhase::Planning;
        task.planning_required = true;
        let mut waiting = snapshot(task.clone());
        waiting.task_prereqs = vec![(TaskId::generate(now()), TaskStatus::Open)];
        let verdict = evaluate_task(&waiting, &person(ActorKind::Agent), now());
        assert!(verdict.can_plan, "dependency waits do not gate planning");
        assert!(!verdict.can_execute);
        assert_eq!(codes(&verdict), vec!["task_prerequisite", "plan_required"]);

        let mut blocked = waiting.clone();
        blocked.task.block = Some(block());
        assert!(!evaluate_task(&blocked, &person(ActorKind::Agent), now()).can_plan);

        let mut proposed = waiting.clone();
        proposed.task.status = TaskStatus::Proposed;
        assert!(!evaluate_task(&proposed, &person(ActorKind::Agent), now()).can_plan);
    }

    #[test]
    fn claim_expiry_is_compared_to_now() {
        let mut snapshot = snapshot(task());
        // Expiring exactly now is inactive (plan/04: expiration is <= now).
        snapshot.active_claim = Some(claim(now()));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.can_execute);
        assert!(codes(&verdict).is_empty());

        snapshot.active_claim = Some(claim(ts("2026-09-14T00:05:00Z")));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(!verdict.can_execute);
        assert_eq!(codes(&verdict), vec!["claimed"]);
        assert_eq!(
            verdict.reasons[0].resource_id,
            snapshot.active_claim.as_ref().unwrap().id
        );
    }

    #[test]
    fn plan_gate_requires_selected_and_accepted_revision() {
        let mut task = task();
        task.planning_required = true;
        let selected = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let mut snapshot = snapshot(task);
        assert!(!evaluate_task(&snapshot, &person(ActorKind::Human), now()).can_execute);

        snapshot.selected_plan_revision_id = Some(selected);
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(!verdict.can_execute, "selection without acceptance");
        assert_eq!(codes(&verdict), vec!["plan_required"]);

        snapshot.accepted_plan_revision_ids =
            vec![Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext))];
        assert!(!evaluate_task(&snapshot, &person(ActorKind::Human), now()).can_execute);

        snapshot.accepted_plan_revision_ids = vec![selected];
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.can_execute);
        assert!(codes(&verdict).is_empty());
    }

    #[test]
    fn human_review_uses_can_review_directly() {
        let producer = ActorId::generate(now());
        let mut task = task();
        task.phase = TaskPhase::WorkReview;
        let mut snapshot = snapshot(task);
        snapshot.pending_submission = Some(submission("work", ReviewPolicy::Human, producer));
        let human = person(ActorKind::Human);
        let verdict = evaluate_task(&snapshot, &human, now());
        assert!(verdict.can_review);
        assert_eq!(codes(&verdict), vec!["work_review_required"]);
        assert!(verdict.allowed_actions.contains(&"reviewSubmission"));
        assert!(!verdict.allowed_actions.contains(&"claimTask"));

        let agent = person(ActorKind::Agent);
        let verdict = evaluate_task(&snapshot, &agent, now());
        assert!(!verdict.can_review);
        assert_eq!(
            codes(&verdict),
            vec!["work_review_required", "reviewer_policy"]
        );
    }

    #[test]
    fn agent_review_excludes_the_producer() {
        let producer = person(ActorKind::Agent);
        let reviewer = person(ActorKind::Agent);
        let mut task = task();
        task.phase = TaskPhase::PlanReview;
        let mut snapshot = snapshot(task);
        snapshot.pending_submission = Some(submission("plan", ReviewPolicy::Agent, producer.id));

        let verdict = evaluate_task(&snapshot, &reviewer, now());
        assert!(verdict.can_review);
        assert_eq!(codes(&verdict), vec!["plan_review_required"]);
        assert!(verdict.allowed_actions.contains(&"claimTask"));
        assert!(!verdict.allowed_actions.contains(&"reviewSubmission"));

        let verdict = evaluate_task(&snapshot, &producer, now());
        assert!(!verdict.can_review);
        assert_eq!(
            codes(&verdict),
            vec!["plan_review_required", "producer_cannot_review"]
        );

        let human = person(ActorKind::Human);
        let verdict = evaluate_task(&snapshot, &human, now());
        assert!(!verdict.can_review, "human does not match agent policy");
    }

    #[test]
    fn mismatched_pending_submission_reports_wrong_phase() {
        let mut task = task();
        task.phase = TaskPhase::Execution;
        let mut snapshot = snapshot(task);
        snapshot.pending_submission = Some(submission(
            "plan",
            ReviewPolicy::Human,
            ActorId::generate(now()),
        ));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(!verdict.can_review);
        assert_eq!(codes(&verdict), vec!["wrong_phase"]);
    }

    #[test]
    fn review_phase_without_submission_still_reports_the_gate() {
        let mut task = task();
        task.phase = TaskPhase::PlanReview;
        let expected = task.id.as_uuid();
        let verdict = evaluate_task(&snapshot(task), &person(ActorKind::Human), now());
        assert!(!verdict.can_review);
        assert_eq!(codes(&verdict), vec!["plan_review_required"]);
        assert_eq!(verdict.reasons[0].resource_id, expected);
    }

    #[test]
    fn select_task_plan_respects_claims_and_pending_reviews() {
        let mut task = task();
        task.planning_required = true;
        task.phase = TaskPhase::Planning;
        let owner = person(ActorKind::Human);
        let base = snapshot(task);
        assert!(
            evaluate_task(&base, &owner, now())
                .allowed_actions
                .contains(&"selectTaskPlan")
        );

        let mut plan_claimed = base.clone();
        let mut plan_claim = claim(ts("2026-09-14T00:05:00Z"));
        plan_claim.phase = "plan".to_string();
        plan_claim.actor_id = owner.id;
        plan_claimed.active_claim = Some(plan_claim);
        assert!(
            evaluate_task(&plan_claimed, &owner, now())
                .allowed_actions
                .contains(&"selectTaskPlan"),
            "plan claimant may save/select its own output"
        );
        // Only the claimant gets the button while the plan claim is active.
        let bystander = person(ActorKind::Human);
        assert!(
            !evaluate_task(&plan_claimed, &bystander, now())
                .allowed_actions
                .contains(&"selectTaskPlan")
        );

        let mut execute_claimed = base.clone();
        execute_claimed.active_claim = Some(claim(ts("2026-09-14T00:05:00Z")));
        assert!(
            !evaluate_task(&execute_claimed, &owner, now())
                .allowed_actions
                .contains(&"selectTaskPlan")
        );

        let mut pending_review = base.clone();
        pending_review.pending_submission = Some(submission(
            "plan",
            ReviewPolicy::Human,
            ActorId::generate(now()),
        ));
        assert!(
            !evaluate_task(&pending_review, &owner, now())
                .allowed_actions
                .contains(&"selectTaskPlan")
        );
    }

    #[test]
    fn agents_cannot_link_proposed_dependents() {
        let mut task = task();
        task.status = TaskStatus::Proposed;
        let snapshot = snapshot(task);
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Agent), now());
        assert!(!verdict.allowed_actions.contains(&"createDependency"));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.allowed_actions.contains(&"createDependency"));
        assert!(verdict.allowed_actions.contains(&"acceptTask"));
    }

    #[test]
    fn agents_cannot_link_dependents_under_proposed_epics() {
        // create_dependency rejects agents when the owning epic is proposed
        // (task_endpoint), so the action must not be advertised either.
        let mut snapshot = snapshot(task());
        snapshot.epic_status = EpicStatus::Proposed;
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Agent), now());
        assert!(!verdict.allowed_actions.contains(&"createDependency"));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.allowed_actions.contains(&"createDependency"));
    }

    #[test]
    fn blocked_task_offers_unblock_to_the_owner_only() {
        let mut task = task();
        task.block = Some(block());
        let snapshot = snapshot(task);
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.allowed_actions.contains(&"unblockTask"));
        assert!(!verdict.allowed_actions.contains(&"blockTask"));
        let verdict = evaluate_task(&snapshot, &person(ActorKind::Agent), now());
        assert!(!verdict.allowed_actions.contains(&"unblockTask"));
    }

    fn epic() -> Epic {
        Epic {
            id: EpicId::generate(now()),
            revision: Revision::INITIAL,
            created_at: now(),
            updated_at: now(),
            project_id: ProjectId::generate(now()),
            goal_id: crate::model::GoalId::generate(now()),
            title: "E".to_string(),
            description: String::new(),
            status: EpicStatus::Open,
            archived: false,
            task_counts: Counts::ZERO,
            block: None,
            archive: None,
            cancellation: None,
        }
    }

    fn epic_snapshot(epic: Epic) -> EpicSnapshot {
        EpicSnapshot {
            epic,
            goal_archived: false,
            project_archived: false,
            task_counts: Counts::ZERO,
            active_descendant_work: false,
            epic_prereqs: Vec::new(),
        }
    }

    #[test]
    fn open_epic_with_no_gates_plans_and_executes() {
        let verdict = evaluate_epic(&epic_snapshot(epic()), &person(ActorKind::Human), now());
        assert!(verdict.can_plan);
        assert!(verdict.can_execute);
        assert!(!verdict.can_review);
        assert!(verdict.reasons.is_empty());
        // Empty epic: explicit completion is offered to the owner (plan/04).
        assert_eq!(
            verdict.allowed_actions,
            vec![
                "updateEpic",
                "blockEpic",
                "cancelEpic",
                "completeEpic",
                "createDependency"
            ]
        );
    }

    #[test]
    fn epic_prerequisites_gate_execute_but_not_plan() {
        let mut snapshot = epic_snapshot(epic());
        snapshot.epic_prereqs = vec![(EpicId::generate(now()), EpicStatus::Active)];
        let verdict = evaluate_epic(&snapshot, &person(ActorKind::Human), now());
        assert!(verdict.can_plan);
        assert!(!verdict.can_execute);
        assert_eq!(codes(&verdict), vec!["epic_prerequisite"]);
        assert!(!verdict.allowed_actions.contains(&"completeEpic"));
    }

    #[test]
    fn proposed_and_blocked_epics_gate_everything() {
        let mut proposed = epic();
        proposed.status = EpicStatus::Proposed;
        let verdict = evaluate_epic(&epic_snapshot(proposed), &person(ActorKind::Agent), now());
        assert!(!verdict.can_plan && !verdict.can_execute);
        assert_eq!(codes(&verdict), vec!["proposal_required"]);
        assert!(!verdict.allowed_actions.contains(&"createDependency"));

        let mut blocked = epic();
        blocked.block = Some(block());
        let verdict = evaluate_epic(&epic_snapshot(blocked), &person(ActorKind::Human), now());
        assert!(!verdict.can_plan && !verdict.can_execute);
        assert_eq!(codes(&verdict), vec!["explicit_block"]);
        assert!(verdict.allowed_actions.contains(&"unblockEpic"));
    }

    #[test]
    fn terminal_and_archived_epics_exit_early() {
        let mut done = epic();
        done.status = EpicStatus::Done;
        let verdict = evaluate_epic(&epic_snapshot(done), &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["terminal"]);
        assert_eq!(verdict.allowed_actions, vec!["archiveEpic"]);

        let mut archived = epic_snapshot(epic());
        archived.goal_archived = true;
        let expected = archived.epic.goal_id.as_uuid();
        let verdict = evaluate_epic(&archived, &person(ActorKind::Human), now());
        assert_eq!(codes(&verdict), vec!["archived"]);
        assert_eq!(verdict.reasons[0].resource_id, expected);
        assert!(verdict.allowed_actions.is_empty());
    }

    #[test]
    fn descendant_work_blocks_epic_dependency_and_archive_buttons() {
        let mut busy = epic_snapshot(epic());
        busy.active_descendant_work = true;
        let verdict = evaluate_epic(&busy, &person(ActorKind::Human), now());
        assert!(!verdict.allowed_actions.contains(&"createDependency"));

        let mut done_busy = busy.clone();
        done_busy.epic.status = EpicStatus::Done;
        let verdict = evaluate_epic(&done_busy, &person(ActorKind::Human), now());
        assert!(!verdict.allowed_actions.contains(&"archiveEpic"));
    }

    #[test]
    fn complete_epic_is_offered_for_all_waived_work_only() {
        let mut all_waived = epic_snapshot(epic());
        all_waived.task_counts = Counts {
            total: 2,
            done: 0,
            cancelled: 2,
            waived: 2,
        };
        let verdict = evaluate_epic(&all_waived, &person(ActorKind::Human), now());
        assert!(verdict.allowed_actions.contains(&"completeEpic"));

        let mut in_progress = epic_snapshot(epic());
        in_progress.task_counts = Counts {
            total: 2,
            done: 1,
            cancelled: 0,
            waived: 0,
        };
        let verdict = evaluate_epic(&in_progress, &person(ActorKind::Human), now());
        assert!(!verdict.allowed_actions.contains(&"completeEpic"));

        let agent = person(ActorKind::Agent);
        let verdict = evaluate_epic(&all_waived, &agent, now());
        assert!(!verdict.allowed_actions.contains(&"completeEpic"));
    }

    #[test]
    fn gate_codes_render_contract_strings() {
        let expected = [
            (GateCode::ProposalRequired, "proposal_required"),
            (GateCode::EpicProposalRequired, "epic_proposal_required"),
            (GateCode::ExplicitBlock, "explicit_block"),
            (GateCode::EpicBlock, "epic_block"),
            (GateCode::EpicPrerequisite, "epic_prerequisite"),
            (GateCode::TaskPrerequisite, "task_prerequisite"),
            (GateCode::PlanRequired, "plan_required"),
            (GateCode::PlanReviewRequired, "plan_review_required"),
            (GateCode::WorkReviewRequired, "work_review_required"),
            (GateCode::Claimed, "claimed"),
            (GateCode::Terminal, "terminal"),
            (GateCode::Archived, "archived"),
            (GateCode::WrongPhase, "wrong_phase"),
            (GateCode::ReviewerPolicy, "reviewer_policy"),
            (GateCode::ProducerCannotReview, "producer_cannot_review"),
        ];
        for (code, text) in expected {
            assert_eq!(code.as_str(), text);
        }
    }
}
