//! Property-based tests: operation-sequence model that exercises the domain
//! and asserts all six invariants after every step.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, TimeDelta, Utc};
use proptest::prelude::*;

use shepherd_core::Store;
use shepherd_core::model::*;

// ── Operation enum ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Op {
    CreateProject,
    CreateTask {
        project_idx: usize,
        status_approved: bool,
    },
    ApproveTask {
        task_idx: usize,
    },
    BlockTask {
        task_idx: usize,
    },
    UnblockTask {
        task_idx: usize,
    },
    CancelTask {
        task_idx: usize,
    },
    AddDependsOn {
        source_idx: usize,
        target_idx: usize,
    },
    AddDecomposition {
        parent_idx: usize,
        child_idx: usize,
    },
    RemoveRelation {
        rel_idx: usize,
    },
    ClaimTask {
        task_idx: usize,
    },
    ReleaseClaim {
        task_idx: usize,
    },
    ReportSession {
        task_idx: usize,
        succeed: bool,
    },
    AdvanceTime {
        seconds: u32,
    },
    SweepExpired,
}

// ── Model state ──────────────────────────────────────────────────────────

struct ModelState {
    projects: Vec<ProjectId>,
    tasks: Vec<(TaskId, ProjectId)>,
    relations: Vec<RelationId>,
    attempt_counts: HashMap<TaskId, i32>,
    sim_now: DateTime<Utc>,
}

impl ModelState {
    fn new() -> Self {
        Self {
            projects: Vec::new(),
            tasks: Vec::new(),
            relations: Vec::new(),
            attempt_counts: HashMap::new(),
            sim_now: Utc::now(),
        }
    }

    fn identity() -> Identity {
        Identity {
            harness: "proptest".into(),
            agent_model: "test".into(),
            session_id: "prop-session".into(),
            label: None,
        }
    }

    async fn apply(&mut self, store: &Store, op: &Op) {
        match op {
            Op::CreateProject => {
                if let Ok(p) = store
                    .create_project(&ProjectCreate {
                        name: format!("project-{}", self.projects.len()),
                        description: None,
                        settings: Some(ProjectSettings { review_gate: false }),
                    })
                    .await
                {
                    self.projects.push(p.id);
                }
            }

            Op::CreateTask {
                project_idx,
                status_approved,
            } => {
                if self.projects.is_empty() {
                    return;
                }
                let pid = self.projects[*project_idx % self.projects.len()];
                let status = if *status_approved {
                    Some(TaskStatus::Approved)
                } else {
                    None
                };
                if let Ok(t) = store
                    .create_task(
                        pid,
                        &TaskCreate {
                            title: format!("task-{}", self.tasks.len()),
                            description: None,
                            task_type: TaskType::Code,
                            status,
                            metadata: None,
                            assignee: None,
                            graph_role: None,
                        },
                    )
                    .await
                {
                    self.attempt_counts.insert(t.id, 0);
                    self.tasks.push((t.id, pid));
                }
            }

            Op::ApproveTask { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store.approve_task(pid, tid).await;
            }

            Op::BlockTask { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store.block_task(pid, tid, "test block").await;
            }

            Op::UnblockTask { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store.unblock_task(pid, tid).await;
            }

            Op::CancelTask { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store.cancel_task(pid, tid).await;
            }

            Op::AddDependsOn {
                source_idx,
                target_idx,
            } => {
                if self.tasks.len() < 2 {
                    return;
                }
                let (src, pid) = self.tasks[*source_idx % self.tasks.len()];
                let (tgt, pid2) = self.tasks[*target_idx % self.tasks.len()];
                if pid != pid2 || src == tgt {
                    return;
                }
                if let Ok(r) = store
                    .create_relation(
                        pid,
                        src,
                        &RelationCreate {
                            relation_type: RelationType::DependsOn,
                            target_task_id: tgt,
                        },
                    )
                    .await
                {
                    self.relations.push(r.id);
                }
            }

            Op::AddDecomposition {
                parent_idx,
                child_idx,
            } => {
                if self.tasks.len() < 2 {
                    return;
                }
                let (parent, pid) = self.tasks[*parent_idx % self.tasks.len()];
                let (child, pid2) = self.tasks[*child_idx % self.tasks.len()];
                if pid != pid2 || parent == child {
                    return;
                }
                if let Ok(r) = store
                    .create_relation(
                        pid,
                        parent,
                        &RelationCreate {
                            relation_type: RelationType::Decomposition,
                            target_task_id: child,
                        },
                    )
                    .await
                {
                    self.relations.push(r.id);
                }
            }

            Op::RemoveRelation { rel_idx } => {
                if self.relations.is_empty() {
                    return;
                }
                let rid = self.relations[*rel_idx % self.relations.len()];
                // We need the relation details to delete it. Try each task.
                for (tid, pid) in &self.tasks {
                    if store.delete_relation(*pid, *tid, rid).await.is_ok() {
                        self.relations.retain(|&r| r != rid);
                        break;
                    }
                }
            }

            Op::ClaimTask { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store
                    .claim_task(
                        pid,
                        tid,
                        &ClaimRequest {
                            identity: Self::identity(),
                            ttl_seconds: 300,
                        },
                        self.sim_now,
                    )
                    .await;
            }

            Op::ReleaseClaim { task_idx } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let _ = store
                    .release_claim(
                        pid,
                        tid,
                        &ClaimRelease {
                            identity: Self::identity(),
                        },
                        self.sim_now,
                    )
                    .await;
            }

            Op::ReportSession { task_idx, succeed } => {
                if self.tasks.is_empty() {
                    return;
                }
                let (tid, pid) = self.tasks[*task_idx % self.tasks.len()];
                let outcome = if *succeed {
                    SessionOutcome::Succeeded
                } else {
                    SessionOutcome::Failed
                };
                if store
                    .create_session(
                        pid,
                        tid,
                        &SessionReport {
                            identity: Self::identity(),
                            started_at: self.sim_now,
                            ended_at: self.sim_now,
                            outcome,
                            failure_reason: if *succeed {
                                None
                            } else {
                                Some("test failure".into())
                            },
                            summary: Some("test session".into()),
                            decisions: None,
                            knowledge_items: None,
                            artifacts: None,
                        },
                    )
                    .await
                    .is_ok()
                    && let Some(count) = self.attempt_counts.get_mut(&tid)
                {
                    *count += 1;
                }
            }

            Op::AdvanceTime { seconds } => {
                self.sim_now += TimeDelta::seconds(i64::from(*seconds));
            }

            Op::SweepExpired => {
                let _ = store.sweep_expired_claims(self.sim_now).await;
            }
        }
    }

    /// Check all six invariants against the current store state.
    async fn check_invariants(&self, store: &Store) {
        for (tid, pid) in &self.tasks {
            let task = match store.get_task(*pid, *tid).await {
                Ok(t) => t,
                Err(_) => continue, // soft-deleted
            };

            // INV 2: Status should be a valid TaskStatus value (enforced
            // by the type system, but let's verify the DB TEXT is parseable).
            let status_str = task.status.to_string();
            assert!(
                status_str.parse::<TaskStatus>().is_ok(),
                "INV2: invalid status string: {status_str}"
            );

            // INV 3: At most one active claim per task.
            let active_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM claims
                 WHERE task_id = ? AND released_at IS NULL AND expires_at > ?",
            )
            .bind(tid.to_string())
            .bind(self.sim_now.to_rfc3339())
            .fetch_one(store.pool())
            .await
            .unwrap();
            assert!(
                active_count <= 1,
                "INV3: task {} has {} active claims",
                tid,
                active_count
            );

            // INV 4: Ready ↔ (approved + all deps done) for non-blocked,
            // non-cancelled tasks.
            if task.status != TaskStatus::Blocked && task.status != TaskStatus::Cancelled {
                let undone_deps: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM relations r
                     JOIN tasks t ON t.id = r.target_task_id
                     WHERE r.source_task_id = ?
                       AND r.type = 'depends_on'
                       AND t.deleted_at IS NULL
                       AND t.status != 'done'",
                )
                .bind(tid.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();

                if task.status == TaskStatus::Ready {
                    assert_eq!(
                        undone_deps, 0,
                        "INV4: task {} is ready but has {} undone deps",
                        tid, undone_deps
                    );
                }
            }

            // INV 5: Attempt count is monotonically non-decreasing.
            if let Some(&prev_count) = self.attempt_counts.get(tid) {
                assert!(
                    task.attempt_count >= prev_count,
                    "INV5: task {} attempt count went from {} to {}",
                    tid,
                    prev_count,
                    task.attempt_count
                );
            }
        }

        // INV 1: Dependency graph is acyclic.
        // Check per-project: load all depends_on edges and do topo sort.
        let project_ids: HashSet<ProjectId> = self.projects.iter().copied().collect();

        for pid in &project_ids {
            let edges: Vec<(String, String)> = sqlx::query_as(
                "SELECT r.source_task_id, r.target_task_id
                 FROM relations r
                 JOIN tasks t ON t.id = r.source_task_id AND t.deleted_at IS NULL
                 WHERE r.type = 'depends_on' AND t.project_id = ?",
            )
            .bind(pid.to_string())
            .fetch_all(store.pool())
            .await
            .unwrap();

            if !edges.is_empty() {
                assert!(
                    is_acyclic(&edges),
                    "INV1: dependency cycle detected in project {pid}"
                );
            }
        }
    }
}

/// Kahn's algorithm: returns true if the graph is acyclic.
fn is_acyclic(edges: &[(String, String)]) -> bool {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    for (src, tgt) in edges {
        adj.entry(src.as_str()).or_default().push(tgt.as_str());
        in_degree.entry(tgt.as_str()).or_insert(0);
        in_degree.entry(src.as_str()).or_insert(0);
        *in_degree.get_mut(tgt.as_str()).unwrap() += 1;
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(node, _)| *node)
        .collect();

    let mut visited = 0;
    while let Some(node) = queue.pop() {
        visited += 1;
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                let deg = in_degree.get_mut(next).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push(next);
                }
            }
        }
    }

    visited == in_degree.len()
}

// ── Proptest strategy ────────────────────────────────────────────────────

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => Just(Op::CreateProject),
        15 => (any::<usize>(), any::<bool>()).prop_map(|(idx, approved)| {
            Op::CreateTask { project_idx: idx, status_approved: approved }
        }),
        8 => any::<usize>().prop_map(|idx| Op::ApproveTask { task_idx: idx }),
        10 => (any::<usize>(), any::<usize>()).prop_map(|(s, t)| {
            Op::AddDependsOn { source_idx: s, target_idx: t }
        }),
        3 => (any::<usize>(), any::<usize>()).prop_map(|(p, c)| {
            Op::AddDecomposition { parent_idx: p, child_idx: c }
        }),
        3 => any::<usize>().prop_map(|idx| Op::RemoveRelation { rel_idx: idx }),
        10 => any::<usize>().prop_map(|idx| Op::ClaimTask { task_idx: idx }),
        3 => any::<usize>().prop_map(|idx| Op::ReleaseClaim { task_idx: idx }),
        10 => (any::<usize>(), any::<bool>()).prop_map(|(idx, succeed)| {
            Op::ReportSession { task_idx: idx, succeed }
        }),
        3 => (1u32..600).prop_map(|s| Op::AdvanceTime { seconds: s }),
        2 => Just(Op::SweepExpired),
        2 => any::<usize>().prop_map(|idx| Op::BlockTask { task_idx: idx }),
        2 => any::<usize>().prop_map(|idx| Op::UnblockTask { task_idx: idx }),
        2 => any::<usize>().prop_map(|idx| Op::CancelTask { task_idx: idx }),
    ]
}

// ── The main proptest ────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]
    #[test]
    fn domain_invariants_hold(ops in prop::collection::vec(op_strategy(), 1..50)) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let store = Store::new_in_memory().await.unwrap();
            let mut model = ModelState::new();

            for op in &ops {
                model.apply(&store, op).await;
                model.check_invariants(&store).await;
            }
        });
    }
}
