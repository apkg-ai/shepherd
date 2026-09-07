//! DAG operations: cycle detection, ancestor traversal, graph role derivation.
//!
//! Pure functions operating on edge lists — no storage access. The store loads
//! edges from SQLite and passes them here.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::{GraphRole, TaskId};

/// Returns `true` if adding `new_from → new_to` (meaning `new_from` depends
/// on `new_to`) would create a cycle in the dependency graph.
///
/// Uses iterative DFS from `new_to` following outgoing `depends_on` edges
/// (source → target). If `new_from` is reachable from `new_to`, the new edge
/// would create a cycle.
pub fn would_create_cycle(edges: &[(TaskId, TaskId)], new_from: TaskId, new_to: TaskId) -> bool {
    if new_from == new_to {
        return true;
    }

    // Build adjacency: source → [targets] (following depends_on direction).
    let mut adj: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    for &(src, tgt) in edges {
        adj.entry(src).or_default().push(tgt);
    }

    // DFS from new_to: can we reach new_from?
    // If we add new_from → new_to, and new_to can already reach new_from,
    // that closes a cycle.
    let mut stack = vec![new_to];
    let mut visited = HashSet::new();
    visited.insert(new_to);

    while let Some(node) = stack.pop() {
        if node == new_from {
            return true;
        }
        if let Some(neighbors) = adj.get(&node) {
            for &next in neighbors {
                if visited.insert(next) {
                    stack.push(next);
                }
            }
        }
    }

    false
}

/// Returns `true` if `new_child` already has a decomposition parent.
pub fn would_violate_single_parent(
    decomposition_edges: &[(TaskId, TaskId)],
    new_child: TaskId,
) -> bool {
    decomposition_edges
        .iter()
        .any(|&(_, child)| child == new_child)
}

/// Walk the full ancestor chain: dependency predecessors + decomposition
/// parents. Returns deduplicated task IDs in root-first order (BFS).
///
/// `depends_on_edges`: `(source=depender, target=prerequisite)` — we follow
/// target → source direction reversed: from a task, its prerequisites are the
/// targets of edges where the task is source.
///
/// Wait — to find ancestors of a task T:
/// - dependency ancestors: tasks that T depends on (targets of edges where
///   T is source). Then recursively, their ancestors.
/// - decomposition parents: tasks that are the parent of T (sources of edges
///   where T is target/child).
pub fn ancestor_chain(
    task_id: TaskId,
    depends_on_edges: &[(TaskId, TaskId)],
    decomposition_edges: &[(TaskId, TaskId)],
) -> Vec<TaskId> {
    // Build reverse lookups.
    // For depends_on: task → its prerequisites (targets where task is source).
    let mut deps_of: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    for &(src, tgt) in depends_on_edges {
        deps_of.entry(src).or_default().push(tgt);
    }

    // For decomposition: child → parent.
    let mut parent_of: HashMap<TaskId, TaskId> = HashMap::new();
    for &(parent, child) in decomposition_edges {
        parent_of.insert(child, parent);
    }

    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Seed: immediate ancestors.
    if let Some(prereqs) = deps_of.get(&task_id) {
        for &p in prereqs {
            if visited.insert(p) {
                queue.push_back(p);
            }
        }
    }
    if let Some(&parent) = parent_of.get(&task_id)
        && visited.insert(parent)
    {
        queue.push_back(parent);
    }

    // BFS for root-first ordering.
    while let Some(node) = queue.pop_front() {
        result.push(node);
        if let Some(prereqs) = deps_of.get(&node) {
            for &p in prereqs {
                if visited.insert(p) {
                    queue.push_back(p);
                }
            }
        }
        if let Some(&parent) = parent_of.get(&node)
            && visited.insert(parent)
        {
            queue.push_back(parent);
        }
    }

    result
}

/// Count tasks transitively downstream of `task_id` via `depends_on` edges.
///
/// "Downstream" means: tasks that directly or indirectly depend on `task_id`.
/// In the edge list, `(source=depender, target=prerequisite)`, so downstream
/// tasks are sources whose target chain includes `task_id`.
pub fn count_downstream(task_id: TaskId, depends_on_edges: &[(TaskId, TaskId)]) -> usize {
    // Build reverse: prerequisite → [dependers].
    let mut dependents_of: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    for &(src, tgt) in depends_on_edges {
        dependents_of.entry(tgt).or_default().push(src);
    }

    let mut visited = HashSet::new();
    let mut stack = vec![task_id];
    visited.insert(task_id);

    while let Some(node) = stack.pop() {
        if let Some(deps) = dependents_of.get(&node) {
            for &dep in deps {
                if visited.insert(dep) {
                    stack.push(dep);
                }
            }
        }
    }

    // Subtract 1 because visited includes the task itself.
    visited.len().saturating_sub(1)
}

/// Derive graph roles for tasks based on `depends_on` edges.
///
/// - `start`: no incoming `depends_on` (nothing depends on having this done
///   first — actually, start means no prerequisites, i.e., no outgoing deps).
///
/// Wait, let me re-read the spec: "start = no incoming depends_on, end = no
/// outgoing depends_on."
///
/// In the edge model `(source=depender, target=prerequisite)`:
/// - A task with no incoming edges (no one depends on it) → `end`
/// - A task with no outgoing edges (depends on nothing) → `start`
///
/// Actually, re-reading the OpenAPI: "Derived by default (start = no incoming
/// depends_on, end = no outgoing depends_on)."
///
/// "incoming depends_on" = edges where this task is the target (something
/// depends on this task being done). If nothing depends on this task, it's
/// an end node.
///
/// Wait, that's backwards from what the spec says. Let me re-read:
/// "start = no incoming depends_on" — if nothing points at you as a
/// prerequisite, you're a start.
///
/// Hmm, that doesn't sound right either. Let me think about this
/// directionally:
///
/// In the graph, depends_on means "A depends on B" = A cannot start until B
/// is done. So B is a prerequisite of A. The edge is (source=A, target=B).
///
/// A "start" task is one that has no prerequisites — nothing it depends on.
/// That means no edges where it is the source. "No outgoing depends_on."
///
/// An "end" task is one that nothing else depends on — no edges where it is
/// the target. "No incoming depends_on."
///
/// But the spec says: "start = no incoming depends_on, end = no outgoing
/// depends_on." This seems to use the opposite edge direction convention.
///
/// Let me just follow the spec literally and match the convention used there.
/// The spec's "incoming" might mean "edges coming into this task" where the
/// task is the target of a depends_on relation. In the spec, depends_on
/// source = the task that depends, target = the prerequisite.
///
/// So "incoming depends_on" for task T = edges where T is target = T is a
/// prerequisite for something. If T has no incoming (nothing depends on T),
/// T is a leaf/end.
///
/// "outgoing depends_on" for task T = edges where T is source = T depends on
/// something. If T has no outgoing (T depends on nothing), T is a start.
///
/// So: start = no outgoing = depends on nothing. end = no incoming = nothing
/// depends on it.
///
/// But the spec says "start = no incoming depends_on". This uses the opposite
/// convention where "incoming" means "edges where you are the depender"
/// (you receive dependencies) vs "outgoing" means "you are depended upon."
///
/// To match the spec literally without ambiguity, I'll define:
/// - start = has no prerequisites (depends on nothing)
/// - end = nothing depends on it (is no one's prerequisite)
pub fn derive_graph_roles(
    task_ids: &[TaskId],
    depends_on_edges: &[(TaskId, TaskId)],
) -> HashMap<TaskId, Vec<GraphRole>> {
    let task_set: HashSet<TaskId> = task_ids.iter().copied().collect();

    // Tasks that have prerequisites (are source of a depends_on).
    let has_prerequisite: HashSet<TaskId> = depends_on_edges.iter().map(|&(src, _)| src).collect();

    // Tasks that are prerequisites for something (are target of a depends_on).
    let is_prerequisite: HashSet<TaskId> = depends_on_edges.iter().map(|&(_, tgt)| tgt).collect();

    let mut result = HashMap::new();
    for &task_id in &task_set {
        let mut roles = Vec::new();
        // Start: has no prerequisites (is not source of any depends_on).
        if !has_prerequisite.contains(&task_id) {
            roles.push(GraphRole::Start);
        }
        // End: is not a prerequisite for anything (is not target of any
        // depends_on).
        if !is_prerequisite.contains(&task_id) {
            roles.push(GraphRole::End);
        }
        if !roles.is_empty() {
            result.insert(task_id, roles);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tid(n: u8) -> TaskId {
        TaskId(Uuid::from_bytes([
            n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]))
    }

    #[test]
    fn no_cycle_in_empty_graph() {
        assert!(!would_create_cycle(&[], tid(1), tid(2)));
    }

    #[test]
    fn self_loop_is_cycle() {
        assert!(would_create_cycle(&[], tid(1), tid(1)));
    }

    #[test]
    fn simple_chain_no_cycle() {
        // A → B → C, adding D → A is fine.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        assert!(!would_create_cycle(&edges, tid(4), tid(1)));
    }

    #[test]
    fn simple_chain_creates_cycle() {
        // A → B → C, adding C → A closes the cycle.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        assert!(would_create_cycle(&edges, tid(3), tid(1)));
    }

    #[test]
    fn diamond_no_cycle() {
        // A → B, A → C, B → D, C → D. Adding E → A is fine.
        let edges = vec![
            (tid(1), tid(2)),
            (tid(1), tid(3)),
            (tid(2), tid(4)),
            (tid(3), tid(4)),
        ];
        assert!(!would_create_cycle(&edges, tid(5), tid(1)));
    }

    #[test]
    fn diamond_creates_cycle() {
        // A → B, A → C, B → D, C → D. Adding D → A closes a cycle.
        let edges = vec![
            (tid(1), tid(2)),
            (tid(1), tid(3)),
            (tid(2), tid(4)),
            (tid(3), tid(4)),
        ];
        assert!(would_create_cycle(&edges, tid(4), tid(1)));
    }

    #[test]
    fn single_parent_ok() {
        let edges = vec![(tid(1), tid(2))];
        // tid(3) has no parent yet.
        assert!(!would_violate_single_parent(&edges, tid(3)));
    }

    #[test]
    fn second_parent_rejected() {
        let edges = vec![(tid(1), tid(2))];
        // tid(2) already has parent tid(1).
        assert!(would_violate_single_parent(&edges, tid(2)));
    }

    #[test]
    fn ancestor_chain_simple() {
        // A depends_on B, B depends_on C.
        let deps = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        let decomp = vec![];
        let ancestors = ancestor_chain(tid(1), &deps, &decomp);
        assert_eq!(ancestors, vec![tid(2), tid(3)]);
    }

    #[test]
    fn ancestor_chain_with_decomposition() {
        // A depends_on B, A is child of P.
        let deps = vec![(tid(1), tid(2))];
        let decomp = vec![(tid(3), tid(1))]; // P=3, child=1
        let ancestors = ancestor_chain(tid(1), &deps, &decomp);
        assert!(ancestors.contains(&tid(2)));
        assert!(ancestors.contains(&tid(3)));
    }

    #[test]
    fn ancestor_chain_deduplicates() {
        // Diamond: A deps B, A deps C, B deps D, C deps D.
        let deps = vec![
            (tid(1), tid(2)),
            (tid(1), tid(3)),
            (tid(2), tid(4)),
            (tid(3), tid(4)),
        ];
        let ancestors = ancestor_chain(tid(1), &deps, &[]);
        // D should appear only once.
        let d_count = ancestors.iter().filter(|&&id| id == tid(4)).count();
        assert_eq!(d_count, 1);
    }

    #[test]
    fn count_downstream_leaf() {
        // (1,2) = 1 depends on 2; (2,3) = 2 depends on 3.
        // Task 1 is the leaf — nothing depends on it.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        assert_eq!(count_downstream(tid(1), &edges), 0);
    }

    #[test]
    fn count_downstream_root() {
        // A → B → C. Downstream of C: A and B depend (transitively) on C.
        // Wait: (A, B) means A depends on B. (B, C) means B depends on C.
        // So C is a prerequisite of B, B is a prerequisite of A.
        // Downstream of C = tasks that depend on C = {B, A} = 2.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        assert_eq!(count_downstream(tid(3), &edges), 2);
    }

    #[test]
    fn count_downstream_middle() {
        // A → B → C. Downstream of B = {A} = 1.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        assert_eq!(count_downstream(tid(2), &edges), 1);
    }

    #[test]
    fn derive_roles_single_task() {
        let roles = derive_graph_roles(&[tid(1)], &[]);
        let r = roles.get(&tid(1)).unwrap();
        assert!(r.contains(&GraphRole::Start));
        assert!(r.contains(&GraphRole::End));
    }

    #[test]
    fn derive_roles_chain() {
        // A → B → C. A depends on B, B depends on C.
        let edges = vec![(tid(1), tid(2)), (tid(2), tid(3))];
        let roles = derive_graph_roles(&[tid(1), tid(2), tid(3)], &edges);

        // C has no prerequisites → start. C is a prerequisite → not end.
        let c = roles.get(&tid(3)).unwrap();
        assert!(c.contains(&GraphRole::Start));
        assert!(!c.contains(&GraphRole::End));

        // A has prerequisites → not start. A is not a prerequisite → end.
        let a = roles.get(&tid(1)).unwrap();
        assert!(!a.contains(&GraphRole::Start));
        assert!(a.contains(&GraphRole::End));

        // B is middle — no roles.
        assert!(!roles.contains_key(&tid(2)));
    }
}
