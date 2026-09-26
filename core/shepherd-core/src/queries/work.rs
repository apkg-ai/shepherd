use sqlx::{QueryBuilder, Row, Sqlite};

use super::{ListParams, Page, decode_after, effective_limit, encode_cursor, split_page};
use crate::error::DomainError;
use crate::model::{Actor, EpicId, ProjectId, Task, TaskId};
use crate::queries::hierarchy::{epic_exists, filter_token, project_exists};
use crate::storage::rows::{format_ts, parse_uuid};
use crate::storage::{StorageError, Store};
use crate::workflow::eligibility::{Eligibility, evaluate_task, load_task_snapshots};

/// Requested work phase (listWork contract); claims store the same vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkPhase {
    Plan,
    Execute,
    Review,
}

impl WorkPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkPhase::Plan => "plan",
            WorkPhase::Execute => "execute",
            WorkPhase::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plan" => Some(WorkPhase::Plan),
            "execute" => Some(WorkPhase::Execute),
            "review" => Some(WorkPhase::Review),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkFilters {
    pub epic_id: Option<EpicId>,
    pub type_key: Option<String>,
}

#[derive(Debug)]
pub struct WorkItem {
    pub task: Task,
    pub eligibility: Eligibility,
}

// The result set is caller-dependent (review policy, producer exclusion), so the
// actor joins the fingerprint: another actor replaying the cursor gets invalid_cursor.
fn work_filter(
    project: &ProjectId,
    phase: WorkPhase,
    filters: &WorkFilters,
    actor: &Actor,
) -> String {
    format!(
        "project={project}&phase={}&epic={}&type_key={}&actor={}",
        phase.as_str(),
        filter_token(filters.epic_id.as_ref()),
        filter_token(filters.type_key.as_deref()),
        actor.id,
    )
}

impl Store {
    /// Only currently eligible tasks for the caller and requested phase, oldest
    /// first (plan/07). Advisory: claiming rechecks eligibility.
    pub async fn list_work(
        &self,
        actor: &Actor,
        project: &ProjectId,
        phase: WorkPhase,
        filters: &WorkFilters,
        params: &ListParams,
    ) -> Result<Page<WorkItem>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown project or epic is 404.
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        if let Some(epic) = filters.epic_id
            && !epic_exists(&mut tx, project, &epic).await?
        {
            return Err(DomainError::NotFound);
        }
        let filter = work_filter(project, phase, filters, actor);
        let mut after = decode_after(params, "listWork", &filter)?;
        let now = self.clock().now();
        let now_text = format_ts(&now);

        // SQL prefilters a strict superset of eligible rows; the evaluator decides.
        // Blocked and actively-claimed candidates are excludable here because
        // every phase's eligibility requires unblocked and unclaimed; pending
        // submissions stay in the superset (review eligibility needs them).
        // The refill loop keysets over candidates while the emitted cursor anchors
        // the last returned item, so post-filter pagination stays deterministic.
        // The scan chunk stays independent of the page limit so a small page over
        // mostly-ineligible rows does not degrade into per-row snapshot loads.
        let chunk = (limit + 1).max(256);
        let mut collected: Vec<WorkItem> = Vec::new();
        loop {
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT id, created_at FROM tasks WHERE project_id = ");
            builder.push_bind(project.to_string());
            builder.push(" AND archived = 0 AND status IN ('open', 'active')");
            builder
                .push(
                    " AND block_actor_id IS NULL AND NOT EXISTS (SELECT 1 FROM claims c \
                 WHERE c.task_id = tasks.id AND c.status = 'active' AND c.expires_at > ",
                )
                .push_bind(now_text.clone())
                .push(")");
            match phase {
                WorkPhase::Plan => builder.push(" AND phase = 'planning'"),
                WorkPhase::Execute => builder.push(" AND phase = 'execution'"),
                WorkPhase::Review => builder.push(" AND phase IN ('plan_review', 'work_review')"),
            };
            if let Some(epic) = filters.epic_id {
                builder.push(" AND epic_id = ").push_bind(epic.to_string());
            }
            if let Some(type_key) = &filters.type_key {
                builder.push(" AND type_key = ").push_bind(type_key.clone());
            }
            if let Some((created_at, id)) = &after {
                builder
                    .push(" AND (created_at > ")
                    .push_bind(created_at.clone())
                    .push(" OR (created_at = ")
                    .push_bind(created_at.clone())
                    .push(" AND id > ")
                    .push_bind(id.clone())
                    .push("))");
            }
            builder
                .push(" ORDER BY created_at ASC, id ASC LIMIT ")
                .push_bind(chunk);
            let rows = builder.build().fetch_all(&mut *tx).await?;
            let chunk_len = i64::try_from(rows.len()).expect("chunk fits in i64");
            let Some(last) = rows.last() else {
                break;
            };
            let next_after = {
                let created_at: String = last.try_get("created_at")?;
                let id: String = last.try_get("id")?;
                (created_at, id)
            };
            let mut candidates = Vec::with_capacity(rows.len());
            for row in &rows {
                let id: String = row.try_get("id")?;
                candidates.push(TaskId::from_uuid(parse_uuid("tasks.id", &id)?));
            }
            let mut snapshots = load_task_snapshots(&mut tx, project, &candidates).await?;
            for id in &candidates {
                let snapshot = snapshots
                    .remove(id)
                    .ok_or_else(|| StorageError::Corrupt(format!("tasks.id: {id}")))?;
                let eligibility = evaluate_task(&snapshot, actor, now);
                let eligible = match phase {
                    WorkPhase::Plan => eligibility.can_plan,
                    WorkPhase::Execute => eligibility.can_execute,
                    WorkPhase::Review => eligibility.can_review,
                };
                if eligible {
                    collected.push(WorkItem {
                        task: snapshot.task,
                        eligibility,
                    });
                    if i64::try_from(collected.len()).expect("page fits in i64") > limit {
                        break;
                    }
                }
            }
            if i64::try_from(collected.len()).expect("page fits in i64") > limit
                || chunk_len < chunk
            {
                break;
            }
            after = Some(next_after);
        }
        tx.commit().await.map_err(StorageError::from)?;
        Ok(split_page(collected, limit, |item: &WorkItem| {
            encode_cursor(
                "listWork",
                &filter,
                &format_ts(&item.task.created_at),
                &item.task.id.to_string(),
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::commands::CommandContext;
    use crate::error::DomainError;
    use crate::model::{
        Actor, ActorKind, Clock, DependencyCreate, EpicCreate, GoalCreate, GoalId, ProjectCreate,
        TestClock,
    };
    use crate::storage::open;
    use crate::storage::rows::{format_ts, insert_actor};
    use crate::storage::testing::{store_options, test_clock};
    use crate::workflow::eligibility::GateCode;
    use uuid::Uuid;

    struct Fixture {
        _dir: tempfile::TempDir,
        clock: Arc<TestClock>,
        store: Store,
        owner: Actor,
        project: ProjectId,
        epic: EpicId,
    }

    fn person(clock: &TestClock, kind: ActorKind, label: &str) -> Actor {
        Actor {
            id: crate::model::ActorId::generate(clock.now()),
            kind,
            label: label.to_string(),
            revoked: false,
            created_at: clock.now(),
        }
    }

    async fn register(store: &Store, actor: &Actor) {
        let inserted = actor.clone();
        store
            .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &inserted).await }))
            .await
            .unwrap();
    }

    fn ctx(actor: &Actor, clock: &TestClock) -> CommandContext {
        CommandContext {
            actor: actor.clone(),
            command_id: crate::model::CommandId::generate(clock.now()),
            idempotency_key: Uuid::nil(),
            expected_revision: None,
            now: clock.now(),
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let clock = test_clock();
        let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
            .await
            .unwrap();
        let owner = person(&clock, ActorKind::Human, "owner");
        register(&store, &owner).await;
        let project = store
            .create_project(
                ctx(&owner, &clock),
                ProjectCreate {
                    name: "P".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let goal: GoalId = store
            .create_goal(
                ctx(&owner, &clock),
                project,
                GoalCreate {
                    title: "G".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let epic = store
            .create_epic(
                ctx(&owner, &clock),
                project,
                goal,
                EpicCreate {
                    title: "E".to_string(),
                    description: None,
                },
            )
            .await
            .unwrap()
            .value
            .id;
        Fixture {
            _dir: dir,
            clock,
            store,
            owner,
            project,
            epic,
        }
    }

    async fn task(f: &Fixture, title: &str) -> TaskId {
        // Distinct created_at keeps oldest-first ordering observable.
        f.clock.advance(chrono::TimeDelta::milliseconds(2));
        f.store
            .create_task(
                ctx(&f.owner, &f.clock),
                f.project,
                f.epic,
                crate::model::TaskCreate {
                    title: title.to_string(),
                    type_key: "code".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id
    }

    async fn list(
        f: &Fixture,
        actor: &Actor,
        phase: WorkPhase,
        params: &ListParams,
    ) -> Page<WorkItem> {
        f.store
            .list_work(actor, &f.project, phase, &WorkFilters::default(), params)
            .await
            .unwrap()
    }

    fn ids(page: &Page<WorkItem>) -> Vec<TaskId> {
        page.items.iter().map(|item| item.task.id).collect()
    }

    #[tokio::test]
    async fn only_eligible_tasks_appear_oldest_first() {
        let f = fixture().await;
        let ready = task(&f, "ready").await;
        let waiting = task(&f, "waiting").await;
        let prerequisite = task(&f, "prerequisite").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Task {
                    dependent_id: waiting,
                    prerequisite_id: prerequisite,
                },
            )
            .await
            .unwrap();
        let blocked = task(&f, "blocked").await;
        // Direct-SQL fixture: block commands land in step 006.
        sqlx::query(
            "UPDATE tasks SET block_actor_id = ?1, block_reason = 'hold', \
             block_created_at = ?2 WHERE id = ?3",
        )
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(blocked.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();

        let page = list(&f, &f.owner, WorkPhase::Execute, &ListParams::default()).await;
        assert_eq!(ids(&page), vec![ready, prerequisite]);
        assert!(page.next_cursor.is_none());
        assert!(page.items[0].eligibility.can_execute);

        // The waiting task returns once its prerequisite is done.
        sqlx::query("UPDATE tasks SET status = 'done', phase = 'complete' WHERE id = ?1")
            .bind(prerequisite.to_string())
            .execute(f.store.pool())
            .await
            .unwrap();
        let page = list(&f, &f.owner, WorkPhase::Execute, &ListParams::default()).await;
        assert_eq!(ids(&page), vec![ready, waiting]);
    }

    #[tokio::test]
    async fn plan_phase_ignores_dependency_waits() {
        let f = fixture().await;
        let planned = f
            .store
            .create_task(
                ctx(&f.owner, &f.clock),
                f.project,
                f.epic,
                crate::model::TaskCreate {
                    title: "planned".to_string(),
                    type_key: "code".to_string(),
                    planning_required: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let prerequisite = task(&f, "prerequisite").await;
        f.store
            .create_dependency(
                ctx(&f.owner, &f.clock),
                f.project,
                DependencyCreate::Task {
                    dependent_id: planned,
                    prerequisite_id: prerequisite,
                },
            )
            .await
            .unwrap();

        let page = list(&f, &f.owner, WorkPhase::Plan, &ListParams::default()).await;
        assert_eq!(ids(&page), vec![planned]);
        let item = &page.items[0];
        assert!(item.eligibility.can_plan);
        assert!(!item.eligibility.can_execute);
        // Reasons still describe execution dependencies during early planning.
        assert!(
            item.eligibility
                .reasons
                .iter()
                .any(|reason| reason.code == GateCode::TaskPrerequisite)
        );
    }

    #[tokio::test]
    async fn pages_keyset_across_ineligible_rows() {
        let f = fixture().await;
        let mut expected = Vec::new();
        for index in 0..6 {
            let id = task(&f, &format!("t{index}")).await;
            if index % 2 == 0 {
                expected.push(id);
            } else {
                // Direct-SQL fixture: block commands land in step 006.
                sqlx::query(
                    "UPDATE tasks SET block_actor_id = ?1, block_reason = 'hold', \
                     block_created_at = ?2 WHERE id = ?3",
                )
                .bind(f.owner.id.to_string())
                .bind(format_ts(&f.clock.now()))
                .bind(id.to_string())
                .execute(f.store.pool())
                .await
                .unwrap();
            }
        }
        let params = |cursor: Option<String>| ListParams {
            limit: Some(2),
            cursor,
            include_archived: false,
        };
        let page1 = list(&f, &f.owner, WorkPhase::Execute, &params(None)).await;
        assert_eq!(ids(&page1), expected[0..2]);
        let page2 = list(&f, &f.owner, WorkPhase::Execute, &params(page1.next_cursor)).await;
        assert_eq!(ids(&page2), expected[2..3]);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn cursors_bind_to_the_actor_and_route_membership_precedes() {
        let f = fixture().await;
        let agent = person(&f.clock, ActorKind::Agent, "agent");
        register(&f.store, &agent).await;
        for index in 0..3 {
            task(&f, &format!("t{index}")).await;
        }
        let page = list(
            &f,
            &f.owner,
            WorkPhase::Execute,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await;
        let cursor = page.next_cursor.unwrap();

        // The result set is caller-dependent: another actor cannot resume it.
        let err = f
            .store
            .list_work(
                &agent,
                &f.project,
                WorkPhase::Execute,
                &WorkFilters::default(),
                &ListParams {
                    limit: Some(2),
                    cursor: Some(cursor.clone()),
                    include_archived: false,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidCursor(_)));

        let unknown_project = ProjectId::generate(f.clock.now());
        let err = f
            .store
            .list_work(
                &f.owner,
                &unknown_project,
                WorkPhase::Execute,
                &WorkFilters::default(),
                &ListParams::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
        let err = f
            .store
            .list_work(
                &f.owner,
                &f.project,
                WorkPhase::Execute,
                &WorkFilters {
                    epic_id: Some(EpicId::generate(f.clock.now())),
                    ..Default::default()
                },
                &ListParams::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn review_list_is_caller_dependent() {
        let f = fixture().await;
        let producer = person(&f.clock, ActorKind::Agent, "producer");
        let reviewer = person(&f.clock, ActorKind::Agent, "reviewer");
        register(&f.store, &producer).await;
        register(&f.store, &reviewer).await;
        let reviewed = task(&f, "reviewed").await;
        let now = format_ts(&f.clock.now());
        // Direct-SQL fixture: sessions/submissions get commands in steps 010/011;
        // the task's phase transition lands with the report commands in step 011.
        let session_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let mut tx = f.store.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, task_id, claim_id, actor_id, phase, started_at, \
             ended_at, outcome, summary, failure_reason, document_revision_ids, links) \
             VALUES (?1, ?2, 'claim', ?3, 'execute', ?4, ?4, 'succeeded', '', '', '[]', '[]')",
        )
        .bind(&session_id)
        .bind(reviewed.to_string())
        .bind(producer.id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO submissions (id, revision, created_at, updated_at, task_id, kind, \
             producer_id, document_revision_ids, session_id, policy, status, \
             created_context_revision) VALUES (?1, 1, ?2, ?2, ?3, 'work', ?4, '[]', ?5, \
             'agent', 'pending', 1)",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(&now)
        .bind(reviewed.to_string())
        .bind(producer.id.to_string())
        .bind(&session_id)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("UPDATE tasks SET status = 'active', phase = 'work_review' WHERE id = ?1")
            .bind(reviewed.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let page = list(&f, &reviewer, WorkPhase::Review, &ListParams::default()).await;
        assert_eq!(ids(&page), vec![reviewed]);
        assert!(page.items[0].eligibility.can_review);
        // The producer never reviews its own submission; humans miss agent policy.
        let page = list(&f, &producer, WorkPhase::Review, &ListParams::default()).await;
        assert!(page.items.is_empty());
        let page = list(&f, &f.owner, WorkPhase::Review, &ListParams::default()).await;
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn claim_expiry_flips_eligibility_with_the_clock() {
        let f = fixture().await;
        let claimed = task(&f, "claimed").await;
        // Direct-SQL fixture: claim commands land in step 009.
        sqlx::query(
            "INSERT INTO claims (id, task_id, actor_id, phase, acquired_at, expires_at, \
             status, task_revision, lease_hash) VALUES (?1, ?2, ?3, 'execute', ?4, ?5, \
             'active', 1, 'hash')",
        )
        .bind(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
        .bind(claimed.to_string())
        .bind(f.owner.id.to_string())
        .bind(format_ts(&f.clock.now()))
        .bind(format_ts(&(f.clock.now() + chrono::TimeDelta::minutes(5))))
        .execute(f.store.pool())
        .await
        .unwrap();

        let page = list(&f, &f.owner, WorkPhase::Execute, &ListParams::default()).await;
        assert!(page.items.is_empty());
        // GET computes expired claims as inactive without any cleanup mutation.
        f.clock.advance(chrono::TimeDelta::minutes(5));
        let page = list(&f, &f.owner, WorkPhase::Execute, &ListParams::default()).await;
        assert_eq!(ids(&page), vec![claimed]);
    }

    #[tokio::test]
    async fn work_phase_round_trips_contract_strings() {
        for phase in [WorkPhase::Plan, WorkPhase::Execute, WorkPhase::Review] {
            assert_eq!(WorkPhase::parse(phase.as_str()), Some(phase));
        }
        assert_eq!(WorkPhase::parse("complete"), None);
    }
}
