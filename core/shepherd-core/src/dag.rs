//! Pure cycle detection on dependency edge lists — no storage access.
//! Commands load scoped edges inside the serialized write transaction and
//! pass them here (plan/03, plan/05).

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Edges are `(dependent_id, prerequisite_id)`. Adding `dependent → prerequisite`
/// closes a cycle iff `dependent` is already reachable from `prerequisite`
/// along existing dependent → prerequisite edges.
pub(crate) fn would_create_cycle<Id: Copy + Eq + Hash>(
    edges: &[(Id, Id)],
    dependent: Id,
    prerequisite: Id,
) -> bool {
    if dependent == prerequisite {
        return true;
    }
    let mut adjacency: HashMap<Id, Vec<Id>> = HashMap::new();
    for &(from, to) in edges {
        adjacency.entry(from).or_default().push(to);
    }
    // Iterative DFS: no recursion, so contract-bound graphs cannot overflow the stack.
    let mut stack = vec![prerequisite];
    let mut visited = HashSet::from([prerequisite]);
    while let Some(node) = stack.pop() {
        if node == dependent {
            return true;
        }
        if let Some(nexts) = adjacency.get(&node) {
            for &next in nexts {
                if visited.insert(next) {
                    stack.push(next);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_has_no_cycle() {
        assert!(!would_create_cycle::<u8>(&[], 1, 2));
    }

    #[test]
    fn self_loop_is_a_cycle() {
        assert!(would_create_cycle::<u8>(&[], 1, 1));
    }

    #[test]
    fn direct_two_node_loop_is_a_cycle() {
        assert!(would_create_cycle(&[(1u8, 2)], 2, 1));
    }

    #[test]
    fn transitive_chain_closes_a_cycle() {
        // 1 → 2 → 3; adding 3 → 1 closes the loop.
        let edges = [(1u8, 2), (2, 3)];
        assert!(would_create_cycle(&edges, 3, 1));
        assert!(!would_create_cycle(&edges, 4, 1));
    }

    #[test]
    fn diamond_branch_and_join_is_not_a_cycle() {
        // 1 → 2, 1 → 3, 2 → 4, 3 → 4: two paths, no loop.
        let edges = [(1u8, 2), (1, 3), (2, 4), (3, 4)];
        assert!(!would_create_cycle(&edges, 5, 1));
        assert!(would_create_cycle(&edges, 4, 1));
    }

    #[test]
    fn disconnected_components_do_not_interact() {
        let edges = [(1u8, 2), (10, 11)];
        assert!(!would_create_cycle(&edges, 11, 2));
        assert!(would_create_cycle(&edges, 11, 10));
    }
}
