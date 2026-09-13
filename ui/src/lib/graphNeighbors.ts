import type { Relation } from "../api/generated/model";

/**
 * Computes the full transitive "cone" of a selected node — every task
 * reachable upstream or downstream along the given relation type. Used to
 * fade non-neighboring nodes so the blocking chain stands out.
 *
 * For `depends_on`: follows source→target (prerequisites) and
 * target→source (dependents), transitively.
 * For `decomposition`: walks parent chain up and subtree down.
 *
 * Pure function, <1 ms at 50–100 tasks.
 */
export function computeNeighborIds(
  selectedId: string,
  relations: readonly Relation[],
  lensType: "decomposition" | "depends_on",
): Set<string> {
  const neighbors = new Set<string>([selectedId]);
  const typed = relations.filter((r) => r.type === lensType);

  // Build adjacency in both directions for a single traversal pass.
  const forward = new Map<string, string[]>(); // source → targets
  const reverse = new Map<string, string[]>(); // target → sources
  for (const r of typed) {
    if (r.source_task_id === r.target_task_id) continue;
    const fwd = forward.get(r.source_task_id);
    if (fwd) fwd.push(r.target_task_id);
    else forward.set(r.source_task_id, [r.target_task_id]);
    const rev = reverse.get(r.target_task_id);
    if (rev) rev.push(r.source_task_id);
    else reverse.set(r.target_task_id, [r.source_task_id]);
  }

  // Walk forward (depends_on: prerequisites chain; decomposition: children).
  const queue: string[] = [selectedId];
  while (queue.length > 0) {
    const current = queue.pop()!;
    for (const next of forward.get(current) ?? []) {
      if (!neighbors.has(next)) {
        neighbors.add(next);
        queue.push(next);
      }
    }
  }

  // Walk reverse (depends_on: dependents chain; decomposition: parent chain).
  queue.push(selectedId);
  while (queue.length > 0) {
    const current = queue.pop()!;
    for (const next of reverse.get(current) ?? []) {
      if (!neighbors.has(next)) {
        neighbors.add(next);
        queue.push(next);
      }
    }
  }

  return neighbors;
}
