use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Row, SqliteConnection};
use uuid::Uuid;

use super::hierarchy::{epic_row, goal_row, project_row, task_row};
use super::submissions::{review_from_row, submission_from_row};
use crate::commands::{revision_from_row, revision_row};
use crate::error::DomainError;
use crate::model::{
    ActorId, Claim, ClaimPhase, Counts, Dependency, DependencyLevel, DocumentRevision,
    DocumentRevisionId, Epic, Goal, LifecycleRecord, Project, ProjectId, Review, Session,
    SessionOutcome, Submission, Task, TaskId,
};
use crate::queries::claims::claim_from_row;
use crate::queries::graph::dependency_from_row;
use crate::storage::rows::{format_ts, parse_ts, parse_uuid};
use crate::storage::{StorageError, Store};

/// Response budget in UTF-8 JSON bytes (plan/05): 256 KiB.
pub const CONTEXT_BUDGET_BYTES: usize = 262_144;

// Contract caps per ContextBundle section; queries fetch cap+1 to detect more rows.
const DOCUMENTS_CAP: usize = 100;
const RECENT_CAP: usize = 20;
const SIBLING_CLAIMS_CAP: usize = 100;
const DEPENDENCIES_CAP: usize = 2_000;

/// Snapshot context for one task (plan/05); step 015 converts it to the wire
/// ContextBundle. The selected/accepted id vectors are conversion inputs for
/// the wire Task, read in the same snapshot.
#[derive(Debug)]
pub struct ContextBundle {
    pub task: Task,
    pub epic: Epic,
    pub goal: Goal,
    pub project: Project,
    pub selected_plan_revision_ids: Vec<Uuid>,
    pub accepted_plan_submission_ids: Vec<Uuid>,
    pub selected_plan: Vec<DocumentRevision>,
    pub documents: Vec<DocumentRevision>,
    pub submissions: Vec<Submission>,
    pub reviews: Vec<Review>,
    pub sessions: Vec<Session>,
    pub dependencies: Vec<Dependency>,
    pub sibling_claims: Vec<Claim>,
    pub truncated: bool,
    pub continuation_paths: Vec<String>,
}

fn measured<T: Serialize>(item: &T) -> Result<usize, DomainError> {
    Ok(serde_json::to_vec(item)
        .map_err(|err| StorageError::Corrupt(format!("context item: {err}")))?
        .len())
}

fn lifecycle_json(record: Option<&LifecycleRecord>) -> Value {
    match record {
        Some(record) => json!([{
            "actor_id": record.actor_id.to_string(),
            "reason": record.reason,
            "created_at": format_ts(&record.created_at),
        }]),
        None => json!([]),
    }
}

fn counts_json(counts: &Counts) -> Value {
    json!({
        "total": counts.total,
        "done": counts.done,
        "cancelled": counts.cancelled,
        "waived": counts.waived,
    })
}

// Eligibility is actor-relative and computed at wire conversion (step 015);
// the budget measures its fixed skeleton and the handoff records the delta.
fn eligibility_json() -> Value {
    json!({
        "can_plan": true,
        "can_execute": true,
        "can_review": true,
        "reasons": [],
        "allowed_actions": [],
    })
}

fn uuid_strings(ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(Uuid::to_string).collect()
}

// Wire-shaped mandatory entities (core Task/Epic/Goal/Project carry no serde);
// step 015's conversion must keep these field sets identical.
fn mandatory_json(
    bundle_task: &Task,
    epic: &Epic,
    goal: &Goal,
    project: &Project,
    selected_plan_revision_ids: &[Uuid],
    accepted_plan_submission_ids: &[Uuid],
) -> Value {
    let task = json!({
        "id": bundle_task.id.to_string(),
        "revision": bundle_task.revision.value(),
        "created_at": format_ts(&bundle_task.created_at),
        "updated_at": format_ts(&bundle_task.updated_at),
        "project_id": bundle_task.project_id.to_string(),
        "epic_id": bundle_task.epic_id.to_string(),
        "title": bundle_task.title,
        "description": bundle_task.description,
        "type_key": bundle_task.type_key,
        "status": bundle_task.status.as_str(),
        "phase": bundle_task.phase.as_str(),
        "planning_required": bundle_task.planning_required,
        "plan_review": bundle_task.plan_review.as_str(),
        "work_review": bundle_task.work_review.as_str(),
        "selected_plan_revision_ids": uuid_strings(selected_plan_revision_ids),
        "accepted_plan_submission_ids": uuid_strings(accepted_plan_submission_ids),
        "archived": bundle_task.archived,
        "blocks": lifecycle_json(bundle_task.block.as_ref()),
        "waivers": lifecycle_json(bundle_task.waiver.as_ref()),
        "attempt_count": bundle_task.attempt_count,
        "eligibility": eligibility_json(),
        "cancellations": lifecycle_json(bundle_task.cancellation.as_ref()),
        "archives": lifecycle_json(bundle_task.archive.as_ref()),
    });
    let epic = json!({
        "id": epic.id.to_string(),
        "revision": epic.revision.value(),
        "created_at": format_ts(&epic.created_at),
        "updated_at": format_ts(&epic.updated_at),
        "project_id": epic.project_id.to_string(),
        "goal_id": epic.goal_id.to_string(),
        "title": epic.title,
        "description": epic.description,
        "status": epic.status.as_str(),
        "archived": epic.archived,
        "blocks": lifecycle_json(epic.block.as_ref()),
        "eligibility": eligibility_json(),
        "task_counts": counts_json(&epic.task_counts),
        "cancellations": lifecycle_json(epic.cancellation.as_ref()),
        "archives": lifecycle_json(epic.archive.as_ref()),
    });
    let goal = json!({
        "id": goal.id.to_string(),
        "revision": goal.revision.value(),
        "created_at": format_ts(&goal.created_at),
        "updated_at": format_ts(&goal.updated_at),
        "project_id": goal.project_id.to_string(),
        "title": goal.title,
        "description": goal.description,
        "archived": goal.archived,
        "epic_counts": counts_json(&goal.epic_counts),
        "completed": goal.completed,
        "archives": lifecycle_json(goal.archive.as_ref()),
    });
    let project = json!({
        "id": project.id.to_string(),
        "revision": project.revision.value(),
        "created_at": format_ts(&project.created_at),
        "updated_at": format_ts(&project.updated_at),
        "name": project.name,
        "description": project.description,
        "settings": {
            "proposal_gate": project.settings.proposal_gate,
            "planning_required": project.settings.planning_required,
            "plan_review": project.settings.plan_review.as_str(),
            "work_review": project.settings.work_review.as_str(),
        },
        "archived": project.archived,
        "epic_counts": counts_json(&project.epic_counts),
        "archives": lifecycle_json(project.archive.as_ref()),
    });
    // Full bundle skeleton: section keys, flag and paths cost bytes too.
    json!({
        "task": task,
        "epic": epic,
        "goal": goal,
        "project": project,
        "selected_plan": [],
        "documents": [],
        "submissions": [],
        "reviews": [],
        "sessions": [],
        "dependencies": [],
        "sibling_claims": [],
        "truncated": false,
        "continuation_paths": [],
    })
}

// Deterministic item-level fill in fixed section priority: the first item that
// does not fit closes its section and every lower-priority one (plan/05).
struct BudgetFill {
    remaining: usize,
    open: bool,
    truncated: bool,
    continuation_paths: Vec<String>,
}

impl BudgetFill {
    fn take<T: Serialize>(
        &mut self,
        rows: Vec<T>,
        cap: usize,
        paths: &[String],
    ) -> Result<Vec<T>, DomainError> {
        let mut rows = rows;
        let mut cut = rows.len() > cap;
        rows.truncate(cap);
        let mut kept = Vec::with_capacity(rows.len());
        for item in rows {
            if !self.open {
                cut = true;
                break;
            }
            let cost = measured(&item)? + 1;
            if cost > self.remaining {
                self.open = false;
                cut = true;
                break;
            }
            self.remaining -= cost;
            kept.push(item);
        }
        if cut {
            self.truncated = true;
            for path in paths {
                if !self.continuation_paths.contains(path) {
                    self.continuation_paths.push(path.clone());
                }
            }
        }
        Ok(kept)
    }
}

fn session_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Session, DomainError> {
    let id: String = row.try_get("id")?;
    let task_id: String = row.try_get("task_id")?;
    let claim_id: String = row.try_get("claim_id")?;
    let actor_id: String = row.try_get("actor_id")?;
    let phase: String = row.try_get("phase")?;
    let started_at: String = row.try_get("started_at")?;
    let ended_at: String = row.try_get("ended_at")?;
    let outcome: String = row.try_get("outcome")?;
    let document_revision_ids: String = row.try_get("document_revision_ids")?;
    let links: String = row.try_get("links")?;
    let revision_ids: Vec<String> = serde_json::from_str(&document_revision_ids)
        .map_err(|err| StorageError::Corrupt(format!("sessions.document_revision_ids: {err}")))?;
    Ok(Session {
        id: parse_uuid("sessions.id", &id)?,
        task_id: TaskId::from_uuid(parse_uuid("sessions.task_id", &task_id)?),
        claim_id: parse_uuid("sessions.claim_id", &claim_id)?,
        actor_id: ActorId::from_uuid(parse_uuid("sessions.actor_id", &actor_id)?),
        phase: ClaimPhase::parse(&phase)
            .ok_or_else(|| StorageError::Corrupt(format!("sessions.phase: {phase}")))?,
        started_at: parse_ts("sessions.started_at", &started_at)?,
        ended_at: parse_ts("sessions.ended_at", &ended_at)?,
        outcome: SessionOutcome::parse(&outcome)
            .ok_or_else(|| StorageError::Corrupt(format!("sessions.outcome: {outcome}")))?,
        summary: row.try_get("summary")?,
        failure_reason: row.try_get("failure_reason")?,
        document_revision_ids: revision_ids
            .iter()
            .map(|id| parse_uuid("sessions.document_revision_ids", id))
            .collect::<Result<Vec<_>, _>>()?,
        links: serde_json::from_str(&links)
            .map_err(|err| StorageError::Corrupt(format!("sessions.links: {err}")))?,
    })
}

async fn selected_ids(
    conn: &mut SqliteConnection,
    task: &TaskId,
) -> Result<(Vec<Uuid>, Vec<Uuid>), DomainError> {
    let row = sqlx::query(
        "SELECT selected_plan_revision_id, accepted_plan_submission_id FROM tasks WHERE id = ?1",
    )
    .bind(task.to_string())
    .fetch_one(&mut *conn)
    .await?;
    let selected: Option<String> = row.try_get("selected_plan_revision_id")?;
    let accepted: Option<String> = row.try_get("accepted_plan_submission_id")?;
    Ok((
        selected
            .map(|id| parse_uuid("tasks.selected_plan_revision_id", &id).map(|u| vec![u]))
            .transpose()?
            .unwrap_or_default(),
        accepted
            .map(|id| parse_uuid("tasks.accepted_plan_submission_id", &id).map(|u| vec![u]))
            .transpose()?
            .unwrap_or_default(),
    ))
}

pub(crate) async fn task_context(
    conn: &mut SqliteConnection,
    project_id: &ProjectId,
    task_id: &TaskId,
    now_ts: &str,
) -> Result<ContextBundle, DomainError> {
    let task = task_row(conn, project_id, task_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    // A task without its ownership chain is corrupt, not absent.
    let missing = |what: &str| StorageError::Corrupt(format!("context: missing {what}"));
    let epic = epic_row(conn, project_id, &task.epic_id)
        .await?
        .ok_or_else(|| missing("epic"))?;
    let goal = goal_row(conn, project_id, &epic.goal_id)
        .await?
        .ok_or_else(|| missing("goal"))?;
    let project = project_row(conn, project_id)
        .await?
        .ok_or_else(|| missing("project"))?;

    let (selected_plan_revision_ids, accepted_plan_submission_ids) =
        selected_ids(conn, task_id).await?;
    // The pinned selected revision, never the document's mutable latest (plan/05).
    let mut selected_plan = Vec::new();
    for id in &selected_plan_revision_ids {
        let revision = revision_row(conn, project_id, &DocumentRevisionId::from_uuid(*id))
            .await?
            .ok_or_else(|| missing("selected plan revision"))?;
        selected_plan.push(revision);
    }

    let project_text = project_id.to_string();
    let task_text = task_id.to_string();
    let epic_text = task.epic_id.to_string();

    // Latest committed revisions of project- and epic-owned documents, newest first.
    let document_rows = sqlx::query(
        "SELECT dr.id, dr.document_id, dr.number, dr.body, dr.links, dr.actor_id, dr.created_at \
         FROM documents d JOIN document_revisions dr ON dr.id = d.latest_revision_id \
         WHERE d.project_id = ?1 AND ((d.owner_kind = 'project' AND d.owner_id = ?1) \
         OR (d.owner_kind = 'epic' AND d.owner_id = ?2)) \
         ORDER BY d.created_at DESC, d.id DESC LIMIT ?3",
    )
    .bind(&project_text)
    .bind(&epic_text)
    .bind(DOCUMENTS_CAP as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let documents = document_rows
        .iter()
        .map(revision_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let session_rows = sqlx::query(
        "SELECT id, task_id, claim_id, actor_id, phase, started_at, ended_at, outcome, \
         summary, failure_reason, document_revision_ids, links \
         FROM sessions WHERE task_id = ?1 ORDER BY ended_at DESC, id DESC LIMIT ?2",
    )
    .bind(&task_text)
    .bind(RECENT_CAP as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let sessions = session_rows
        .iter()
        .map(session_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let submission_rows = sqlx::query(
        "SELECT id, revision, created_at, updated_at, task_id, kind, producer_id, \
         document_revision_ids, session_id, policy, status, created_context_revision \
         FROM submissions WHERE task_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
    )
    .bind(&task_text)
    .bind(RECENT_CAP as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let submissions = submission_rows
        .iter()
        .map(submission_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let review_rows = sqlx::query(
        "SELECT r.id, r.submission_id, r.actor_id, r.decision, r.reason, r.created_at \
         FROM reviews r JOIN submissions s ON s.id = r.submission_id \
         WHERE s.task_id = ?1 ORDER BY r.created_at DESC, r.id DESC LIMIT ?2",
    )
    .bind(&task_text)
    .bind(RECENT_CAP as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let reviews = review_rows
        .iter()
        .map(review_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    // Sibling awareness: effective-active claims on other tasks of this epic.
    let sibling_rows = sqlx::query(
        "SELECT c.id, c.task_id, c.actor_id, c.phase, c.submission_id, c.acquired_at, \
         c.expires_at, 'active' AS effective_status, c.task_revision, c.plan_revision_id \
         FROM claims c JOIN tasks t ON t.id = c.task_id \
         WHERE t.project_id = ?1 AND t.epic_id = ?2 AND c.task_id <> ?3 \
         AND c.status = 'active' AND c.expires_at > ?4 \
         ORDER BY c.acquired_at DESC, c.id DESC LIMIT ?5",
    )
    .bind(&project_text)
    .bind(&epic_text)
    .bind(&task_text)
    .bind(now_ts)
    .bind(SIBLING_CLAIMS_CAP as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let sibling_claims = sibling_rows
        .iter()
        .map(claim_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    // Epic links touching this epic, then task links touching this task.
    let mut dependencies = Vec::new();
    for (level, table, scope) in [
        (DependencyLevel::Epic, "epic_dependencies", &epic_text),
        (DependencyLevel::Task, "task_dependencies", &task_text),
    ] {
        // AssertSqlSafe: interpolates a compile-time table name only.
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT id, revision, created_at, updated_at, project_id, dependent_id, \
             prerequisite_id FROM {table} WHERE project_id = ?1 \
             AND (dependent_id = ?2 OR prerequisite_id = ?2) \
             ORDER BY created_at ASC, id ASC LIMIT ?3"
        )))
        .bind(&project_text)
        .bind(scope)
        .bind(DEPENDENCIES_CAP as i64 + 1)
        .fetch_all(&mut *conn)
        .await?;
        for row in &rows {
            dependencies.push(dependency_from_row(level, row)?);
        }
    }

    assemble(
        task,
        epic,
        goal,
        project,
        selected_plan_revision_ids,
        accepted_plan_submission_ids,
        selected_plan,
        documents,
        sessions,
        submissions,
        reviews,
        sibling_claims,
        dependencies,
    )
}

#[expect(clippy::too_many_arguments)]
fn assemble(
    task: Task,
    epic: Epic,
    goal: Goal,
    project: Project,
    selected_plan_revision_ids: Vec<Uuid>,
    accepted_plan_submission_ids: Vec<Uuid>,
    selected_plan: Vec<DocumentRevision>,
    documents: Vec<DocumentRevision>,
    sessions: Vec<Session>,
    submissions: Vec<Submission>,
    reviews: Vec<Review>,
    sibling_claims: Vec<Claim>,
    dependencies: Vec<Dependency>,
) -> Result<ContextBundle, DomainError> {
    let prefix = format!("/api/v1/projects/{}", task.project_id);
    let sessions_path = format!("{prefix}/tasks/{}/sessions", task.id);
    // Reviews hang off submissions (one review max per submission), so both
    // sections continue through the submissions collection.
    let submissions_path = format!("{prefix}/submissions?task_id={}", task.id);
    let document_paths = [
        format!(
            "{prefix}/documents?owner_kind=project&owner_id={}",
            task.project_id
        ),
        format!("{prefix}/documents?owner_kind=epic&owner_id={}", epic.id),
    ];
    let claims_path = format!("{prefix}/claims?active=true");
    let dependencies_path = format!("{prefix}/dependencies");

    let mut mandatory = measured(&mandatory_json(
        &task,
        &epic,
        &goal,
        &project,
        &selected_plan_revision_ids,
        &accepted_plan_submission_ids,
    ))?;
    for revision in &selected_plan {
        mandatory += measured(revision)? + 1;
    }
    if mandatory > CONTEXT_BUDGET_BYTES {
        // Return the plan's metadata path rather than silently dropping it (plan/05).
        let continuation = selected_plan_revision_ids
            .first()
            .map(|id| format!("{prefix}/document-revisions/{id}"))
            .unwrap_or_else(|| format!("{prefix}/documents"));
        return Err(DomainError::ContextTooLarge { continuation });
    }

    // Reserve worst-case continuation bytes so emitting paths never overflows.
    let reserve: usize = [
        &sessions_path,
        &submissions_path,
        &document_paths[0],
        &document_paths[1],
        &claims_path,
        &dependencies_path,
    ]
    .iter()
    .map(|path| path.len() + 3)
    .sum();
    let mut fill = BudgetFill {
        remaining: CONTEXT_BUDGET_BYTES
            .saturating_sub(mandatory)
            .saturating_sub(reserve),
        open: true,
        truncated: false,
        continuation_paths: Vec::new(),
    };

    // Fixed priority (plan/05 listing order, unranked sections last).
    let sessions = fill.take(sessions, RECENT_CAP, std::slice::from_ref(&sessions_path))?;
    let reviews = fill.take(reviews, RECENT_CAP, std::slice::from_ref(&submissions_path))?;
    let submissions = fill.take(
        submissions,
        RECENT_CAP,
        std::slice::from_ref(&submissions_path),
    )?;
    let documents = fill.take(documents, DOCUMENTS_CAP, &document_paths)?;
    let sibling_claims = fill.take(
        sibling_claims,
        SIBLING_CLAIMS_CAP,
        std::slice::from_ref(&claims_path),
    )?;
    let dependencies = fill.take(
        dependencies,
        DEPENDENCIES_CAP,
        std::slice::from_ref(&dependencies_path),
    )?;

    Ok(ContextBundle {
        task,
        epic,
        goal,
        project,
        selected_plan_revision_ids,
        accepted_plan_submission_ids,
        selected_plan,
        documents,
        submissions,
        reviews,
        sessions,
        dependencies,
        sibling_claims,
        truncated: fill.truncated,
        continuation_paths: fill.continuation_paths,
    })
}

impl Store {
    pub async fn get_task_context(
        &self,
        project: &ProjectId,
        task: &TaskId,
    ) -> Result<ContextBundle, DomainError> {
        let now_ts = format_ts(&self.clock().now());
        let mut tx = self.read_transaction().await?;
        let bundle = task_context(&mut tx, project, task, &now_ts).await?;
        tx.commit().await.map_err(StorageError::from)?;
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use super::*;
    use crate::commands::CommandContext;
    use crate::model::{
        Actor, ActorKind, Clock, DependencyCreate, Document, DocumentCreate, DocumentKind,
        DocumentOwner, EpicCreate, EpicId, GoalCreate, ProjectCreate, SelectPlanInput, TaskCreate,
        TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::insert_actor;
    use crate::storage::testing::{seed_claim, seed_submission, store_options, test_clock};

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        epic: EpicId,
        task: TaskId,
        sibling: TaskId,
        stale: TaskId,
    }

    fn ctx(actor: &Actor, clock: &TestClock, expected_revision: Option<i64>) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: crate::model::CommandId::generate(clock.now()),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision,
            now: clock.now(),
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind: ActorKind::Human,
            label: "owner".into(),
            revoked: false,
            created_at: clock.now(),
        };
        let registered = owner.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
            .await
            .unwrap();
        let project = store
            .create_project(
                ctx(&owner, &clock, None),
                ProjectCreate {
                    name: "P".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let goal = store
            .create_goal(
                ctx(&owner, &clock, None),
                project,
                GoalCreate {
                    title: "G".into(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        clock.advance(TimeDelta::milliseconds(2));
        let epic = store
            .create_epic(
                ctx(&owner, &clock, None),
                project,
                goal,
                EpicCreate {
                    title: "E".into(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let mut tasks = Vec::new();
        for title in ["T1", "T2", "T3"] {
            clock.advance(TimeDelta::milliseconds(2));
            tasks.push(
                store
                    .create_task(
                        ctx(&owner, &clock, None),
                        project,
                        epic,
                        TaskCreate {
                            title: title.into(),
                            type_key: "code".into(),
                            planning_required: Some(true),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap()
                    .value
                    .id,
            );
        }
        let stale = tasks.pop().unwrap();
        let sibling = tasks.pop().unwrap();
        let task = tasks.pop().unwrap();
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            epic,
            task,
            sibling,
            stale,
        }
    }

    async fn create_document(
        f: &Fixture,
        owner: DocumentOwner,
        kind: DocumentKind,
        body: &str,
    ) -> Document {
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .create_document(
                ctx(&f.owner, &f.clock, None),
                f.project,
                DocumentCreate {
                    owner,
                    kind,
                    title: format!("{} doc", kind.as_str()),
                    body: body.into(),
                    links: Vec::new(),
                },
                "doc-hash",
            )
            .await
            .unwrap()
            .into_inner()
    }

    #[test]
    fn budget_fill_cuts_items_then_sections_deterministically() {
        let path_a = vec!["/a".to_string()];
        let path_b = vec!["/b".to_string(), "/b2".to_string()];
        let mut fill = BudgetFill {
            remaining: 20,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };

        // Three 9-byte items (8 serialized + separator): the third exceeds the
        // 20-byte budget and closes every lower-priority section.
        let items = vec![
            "xxxxxx".to_string(),
            "yyyyyy".to_string(),
            "zzzzzz".to_string(),
        ];
        let kept = fill.take(items, 10, &path_a).unwrap();
        assert_eq!(kept.len(), 2);
        assert!(!fill.open);
        assert!(fill.truncated);
        let lower = fill.take(vec!["a".to_string()], 10, &path_b).unwrap();
        assert!(lower.is_empty());
        assert_eq!(fill.continuation_paths, vec!["/a", "/b", "/b2"]);

        // A count-cap cut keeps lower sections open and deduplicates paths.
        let mut fill = BudgetFill {
            remaining: 1_000,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };
        let kept = fill
            .take(vec!["a".to_string(), "b".to_string()], 1, &path_a)
            .unwrap();
        assert_eq!(kept, vec!["a".to_string()]);
        assert!(fill.open);
        let kept = fill
            .take(vec!["c".to_string(), "d".to_string()], 1, &path_a)
            .unwrap();
        assert_eq!(kept, vec!["c".to_string()]);
        assert_eq!(fill.continuation_paths, vec!["/a"]);
        assert!(fill.truncated);

        // Nothing to cut: no flag, no paths.
        let mut fill = BudgetFill {
            remaining: 1_000,
            open: true,
            truncated: false,
            continuation_paths: Vec::new(),
        };
        fill.take(vec!["a".to_string()], 10, &path_a).unwrap();
        assert!(!fill.truncated);
        assert!(fill.continuation_paths.is_empty());
    }

    #[tokio::test]
    async fn context_assembles_every_section_from_one_snapshot() {
        let f = fixture().await;

        // Pinned plan: select the first revision, then move the latest on.
        let plan = create_document(&f, DocumentOwner::Task(f.task), DocumentKind::Plan, "v1").await;
        let pinned = plan.latest_revision_id.as_uuid();
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .create_document_revision(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                plan.id,
                crate::model::RevisionCreate {
                    body: "v2".into(),
                    links: Vec::new(),
                },
                "rev-hash",
            )
            .await
            .unwrap();
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                f.task,
                SelectPlanInput {
                    document_revision_id: pinned,
                },
            )
            .await
            .unwrap();

        // Scoped documents: project- and this-epic-owned only.
        let brief = create_document(
            &f,
            DocumentOwner::Project(f.project),
            DocumentKind::Brief,
            "b",
        )
        .await;
        let note = create_document(&f, DocumentOwner::Epic(f.epic), DocumentKind::Note, "n").await;

        // Seeded history rows: one submission (with its session) and a review.
        let now_text = format_ts(&f.clock.now());
        let submission =
            seed_submission(f.store.pool(), f.task, f.owner.id, "accepted", &now_text).await;
        sqlx::query(
            "INSERT INTO reviews (id, submission_id, actor_id, decision, reason, created_at) \
             VALUES (?1, ?2, ?3, 'approve', 'fine', ?4)",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(&submission)
        .bind(f.owner.id.to_string())
        .bind(&now_text)
        .execute(f.store.pool())
        .await
        .unwrap();

        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock, None),
                f.project,
                DependencyCreate::Task {
                    dependent_id: f.task,
                    prerequisite_id: f.sibling,
                },
            )
            .await
            .unwrap();

        // Sibling awareness: one live claim, one expired, one on the task itself.
        let live_until = format_ts(&(f.clock.now() + TimeDelta::seconds(300)));
        let lapsed = format_ts(&(f.clock.now() - TimeDelta::seconds(1)));
        let live = seed_claim(
            f.store.pool(),
            f.sibling,
            f.owner.id,
            "execute",
            "active",
            &now_text,
            &live_until,
        )
        .await;
        seed_claim(
            f.store.pool(),
            f.stale,
            f.owner.id,
            "plan",
            "active",
            &now_text,
            &lapsed,
        )
        .await;
        seed_claim(
            f.store.pool(),
            f.task,
            f.owner.id,
            "execute",
            "active",
            &now_text,
            &live_until,
        )
        .await;

        let bundle = f.store.get_task_context(&f.project, &f.task).await.unwrap();

        assert_eq!(bundle.task.id, f.task);
        assert_eq!(bundle.epic.id, f.epic);
        assert_eq!(bundle.selected_plan_revision_ids, vec![pinned]);
        assert_eq!(bundle.selected_plan.len(), 1);
        assert_eq!(bundle.selected_plan[0].body, "v1");
        let document_ids: Vec<Uuid> = bundle
            .documents
            .iter()
            .map(|revision| revision.document_id.as_uuid())
            .collect();
        assert_eq!(document_ids, vec![note.id.as_uuid(), brief.id.as_uuid()]);
        assert_eq!(bundle.submissions.len(), 1);
        assert_eq!(bundle.reviews.len(), 1);
        assert_eq!(bundle.reviews[0].reason, "fine");
        assert_eq!(bundle.sessions.len(), 1);
        assert_eq!(bundle.sibling_claims.len(), 1);
        assert_eq!(bundle.sibling_claims[0].id.to_string(), live);
        assert_eq!(bundle.dependencies.len(), 1);
        assert_eq!(bundle.dependencies[0].level, DependencyLevel::Task);
        assert!(!bundle.truncated);
        assert!(bundle.continuation_paths.is_empty());
    }

    #[tokio::test]
    async fn context_truncates_by_bytes_with_continuation_paths() {
        let f = fixture().await;
        for _ in 0..5 {
            create_document(
                &f,
                DocumentOwner::Epic(f.epic),
                DocumentKind::Note,
                &"x".repeat(60_000),
            )
            .await;
        }

        let bundle = f.store.get_task_context(&f.project, &f.task).await.unwrap();
        assert!(bundle.truncated);
        assert_eq!(bundle.documents.len(), 4, "budget keeps the newest four");
        assert_eq!(
            bundle.continuation_paths,
            vec![
                format!(
                    "/api/v1/projects/{}/documents?owner_kind=project&owner_id={}",
                    f.project, f.project
                ),
                format!(
                    "/api/v1/projects/{}/documents?owner_kind=epic&owner_id={}",
                    f.project, f.epic
                ),
            ]
        );
    }

    #[tokio::test]
    async fn context_rejects_unknown_task_and_oversized_mandatory_plan() {
        let f = fixture().await;
        let unknown = TaskId::generate(f.clock.now());
        assert!(matches!(
            f.store.get_task_context(&f.project, &unknown).await,
            Err(DomainError::NotFound)
        ));

        // 65,536 four-byte characters: the plan alone fills the 256 KiB budget.
        let plan = create_document(
            &f,
            DocumentOwner::Task(f.task),
            DocumentKind::Plan,
            &"\u{1F4A5}".repeat(65_536),
        )
        .await;
        let revision = plan.latest_revision_id.as_uuid();
        f.clock.advance(TimeDelta::milliseconds(2));
        f.store
            .select_task_plan(
                ctx(&f.owner, &f.clock, Some(1)),
                f.project,
                f.task,
                SelectPlanInput {
                    document_revision_id: revision,
                },
            )
            .await
            .unwrap();

        let err = f
            .store
            .get_task_context(&f.project, &f.task)
            .await
            .unwrap_err();
        match err {
            DomainError::ContextTooLarge { continuation } => {
                assert_eq!(
                    continuation,
                    format!(
                        "/api/v1/projects/{}/document-revisions/{revision}",
                        f.project
                    )
                );
            }
            other => panic!("expected ContextTooLarge, got {other:?}"),
        }
    }
}
