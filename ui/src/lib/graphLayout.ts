import dagre from "@dagrejs/dagre";
import { MarkerType, Position, type Edge, type Node } from "@xyflow/react";
import type { Relation, Task } from "../api/generated/model";

/**
 * Pure task-graph → React Flow assembly for the two lenses (docs/04-ui.md):
 * decomposition tree (parent/child, top→bottom) and dependency flow
 * (depends_on DAG, start→end left→right). One dagre rank layout serves both
 * — a tree is a DAG, and dagre handles forests and isolated nodes.
 *
 * Relation semantics (docs/02-domain-model.md, mirrored in taskTree.ts):
 * decomposition source is the parent, target the child; depends_on source is
 * the DEPENDENT, target the prerequisite — so flow edges render
 * target → source to read start → end.
 *
 * Nodes have a fixed size so layout stays pure (no measure loop); node id is
 * the task id (stable across lenses — selection survives a toggle), edge id
 * the relation id.
 */

export const NODE_WIDTH = 248;
export const NODE_HEIGHT = 76;

export type TaskNodeType = Node<{ task: Task }, "task">;

export interface GraphModel {
  nodes: TaskNodeType[];
  edges: Edge[];
}

interface LensEdge {
  relation: Relation;
  /** Rendered direction (already lens-oriented). */
  source: string;
  target: string;
}

/** Decomposition lens: parent above child, top→bottom. */
export function decompositionGraph(
  tasks: readonly Task[],
  relations: readonly Relation[],
): GraphModel {
  return layout(tasks, lensEdges(tasks, relations, "decomposition"), "TB");
}

/** Dependency lens: prerequisite before dependent, start→end left→right. */
export function dependencyGraph(
  tasks: readonly Task[],
  relations: readonly Relation[],
): GraphModel {
  // Flip: depends_on points dependent → prerequisite, the lens reads
  // prerequisite → dependent.
  const edges = lensEdges(tasks, relations, "depends_on").map((e) => ({
    relation: e.relation,
    source: e.relation.target_task_id,
    target: e.relation.source_task_id,
  }));
  return layout(tasks, edges, "LR", { markerEnd: { type: MarkerType.ArrowClosed } });
}

/** Edges of one type whose endpoints are both loaded; self-edges dropped. */
function lensEdges(
  tasks: readonly Task[],
  relations: readonly Relation[],
  type: Relation["type"],
): LensEdge[] {
  const loaded = new Set(tasks.map((t) => t.id));
  return relations
    .filter(
      (r) =>
        r.type === type &&
        r.source_task_id !== r.target_task_id &&
        loaded.has(r.source_task_id) &&
        loaded.has(r.target_task_id),
    )
    .map((relation) => ({
      relation,
      source: relation.source_task_id,
      target: relation.target_task_id,
    }));
}

function layout(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  rankdir: "TB" | "LR",
  edgeExtras: Partial<Edge> = {},
): GraphModel {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir, nodesep: 24, ranksep: 48 });
  g.setDefaultEdgeLabel(() => ({}));

  for (const task of tasks) g.setNode(task.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  for (const edge of edges) g.setEdge(edge.source, edge.target);

  dagre.layout(g);

  const statusById = new Map(tasks.map((t) => [t.id, t.status]));
  const horizontal = rankdir === "LR";

  const nodes: TaskNodeType[] = tasks.map((task) => {
    const placed = g.node(task.id);
    return {
      id: task.id,
      type: "task",
      data: { task },
      // dagre positions node centers; React Flow wants top-left corners.
      position: { x: placed.x - NODE_WIDTH / 2, y: placed.y - NODE_HEIGHT / 2 },
      // Explicit dimensions: edges render immediately, no measure pass.
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      sourcePosition: horizontal ? Position.Right : Position.Bottom,
      targetPosition: horizontal ? Position.Left : Position.Top,
      draggable: false,
      connectable: false,
    };
  });

  const flowEdges: Edge[] = edges.map(({ relation, source, target }) => ({
    id: relation.id,
    source,
    target,
    // A live agent's path pulses: edges touching an in_progress task animate.
    animated: statusById.get(source) === "in_progress" || statusById.get(target) === "in_progress",
    ...edgeExtras,
  }));

  return { nodes, edges: flowEdges };
}
