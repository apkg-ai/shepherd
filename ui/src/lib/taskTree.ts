import type { Relation, Task, TaskStatus } from "../api/generated/model";

/**
 * Builds the task-list hierarchy from per-task relation lists (there is no
 * bulk relations endpoint in v1 — a project-wide one is an S8 candidate, so
 * the screen fetches relations per visible task and this stays pure).
 *
 * Semantics (docs/02-domain-model.md): decomposition source is the parent,
 * target the child; depends_on source is the task that depends on the target.
 */

export interface TaskTreeNode {
  task: Task;
  /** Children present in the loaded task set, in page order. */
  children: TaskTreeNode[];
  /** Total subtasks per this task's own relations — children may be unloaded. */
  subtaskCount: number;
  /** Parent task id, if a decomposition edge names one. */
  parentId: string | null;
  /** True when the parent is part of the loaded task set. */
  parentInView: boolean;
  /** Tasks this one depends on (prerequisites). */
  dependsOnIds: string[];
}

export type RelationsByTask = ReadonlyMap<string, readonly Relation[] | undefined>;

/**
 * Groups loaded tasks under their loaded parents; everything else (roots,
 * tasks with unloaded parents, tasks whose relations are still loading)
 * stays top-level in page order. Cycle-guarded: a task is attached at most
 * once, and self-parents are ignored.
 */
export function buildTaskTree(
  tasks: readonly Task[],
  relationsByTask: RelationsByTask,
): TaskTreeNode[] {
  const nodes = new Map<string, TaskTreeNode>();
  for (const task of tasks) {
    const relations = relationsByTask.get(task.id);
    const parent = relations?.find(
      (r) =>
        r.type === "decomposition" && r.target_task_id === task.id && r.source_task_id !== task.id,
    );
    nodes.set(task.id, {
      task,
      children: [],
      subtaskCount:
        relations?.filter((r) => r.type === "decomposition" && r.source_task_id === task.id)
          .length ?? 0,
      parentId: parent?.source_task_id ?? null,
      parentInView: false,
      dependsOnIds:
        relations
          ?.filter((r) => r.type === "depends_on" && r.source_task_id === task.id)
          .map((r) => r.target_task_id) ?? [],
    });
  }

  const roots: TaskTreeNode[] = [];
  for (const task of tasks) {
    const node = nodes.get(task.id)!;
    const parent = node.parentId !== null ? nodes.get(node.parentId) : undefined;
    // Attach under a loaded parent unless that would close a cycle.
    if (parent && !isAncestor(nodes, task.id, node.parentId!)) {
      node.parentInView = true;
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }
  return roots;
}

/** True if `candidateAncestorOf` sits on `taskId`'s parent chain start. */
function isAncestor(nodes: Map<string, TaskTreeNode>, taskId: string, startId: string): boolean {
  let current: string | null = startId;
  const seen = new Set<string>();
  while (current !== null && !seen.has(current)) {
    if (current === taskId) return true;
    seen.add(current);
    current = nodes.get(current)?.parentId ?? null;
  }
  return false;
}

export type DependencyHint =
  | { kind: "waits_on"; count: number }
  | { kind: "dependencies"; count: number }
  | null;

/**
 * "waits on N" when dependency statuses are known (unmet = not done);
 * "N dependencies" when some target statuses are unknown; null when the
 * task has none or nothing is actually blocking.
 */
export function describeDependencies(
  node: TaskTreeNode,
  statusById: ReadonlyMap<string, TaskStatus>,
): DependencyHint {
  if (node.dependsOnIds.length === 0) return null;
  const statuses = node.dependsOnIds.map((id) => statusById.get(id));
  if (statuses.some((status) => status === undefined)) {
    return { kind: "dependencies", count: node.dependsOnIds.length };
  }
  const unmet = statuses.filter((status) => status !== "done").length;
  return unmet > 0 ? { kind: "waits_on", count: unmet } : null;
}
