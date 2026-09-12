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

/** Gap between packed component blocks. */
const BLOCK_GAP = 80;
/** Row width the component packing wraps at — bounds the canvas aspect. */
const MAX_ROW_WIDTH = 2400;

interface Block {
  /** Positions local to the block (top-left corners, origin-normalized). */
  positions: Map<string, { x: number; y: number }>;
  width: number;
  height: number;
}

/**
 * Dagre lays a forest side-by-side in one endless rank, so a project with
 * many disconnected components (every epic is one, in the tree lens) becomes
 * an unreadable strip. Instead: dagre per connected component, edge-less
 * tasks gathered into one compact grid, then the blocks shelf-packed into
 * rows — the canvas stays near-viewport-shaped at any project size.
 */
function layout(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  rankdir: "TB" | "LR",
  edgeExtras: Partial<Edge> = {},
): GraphModel {
  // Connected components via union-find over the lens edges.
  const root = new Map<string, string>(tasks.map((t) => [t.id, t.id]));
  const find = (id: string): string => {
    let current = id;
    while (root.get(current) !== current) current = root.get(current)!;
    root.set(id, current);
    return current;
  };
  for (const edge of edges) root.set(find(edge.source), find(edge.target));

  const componentTasks = new Map<string, Task[]>();
  for (const task of tasks) {
    const key = find(task.id);
    const group = componentTasks.get(key);
    if (group === undefined) componentTasks.set(key, [task]);
    else group.push(task);
  }

  // Blocks in first-task order; singletons pool into one grid block at the end.
  const blocks: Block[] = [];
  const singles: Task[] = [];
  const seen = new Set<string>();
  for (const task of tasks) {
    const key = find(task.id);
    if (seen.has(key)) continue;
    seen.add(key);
    const group = componentTasks.get(key)!;
    if (group.length === 1) singles.push(task);
    else {
      blocks.push(
        dagreBlock(
          group,
          edges.filter((e) => find(e.source) === key),
          rankdir,
        ),
      );
    }
  }
  if (singles.length > 0) blocks.push(gridBlock(singles));

  // Shelf-pack the blocks into rows.
  const positions = new Map<string, { x: number; y: number }>();
  let x = 0;
  let y = 0;
  let rowHeight = 0;
  for (const block of blocks) {
    if (x > 0 && x + block.width > MAX_ROW_WIDTH) {
      x = 0;
      y += rowHeight + BLOCK_GAP;
      rowHeight = 0;
    }
    for (const [id, local] of block.positions) {
      positions.set(id, { x: local.x + x, y: local.y + y });
    }
    x += block.width + BLOCK_GAP;
    rowHeight = Math.max(rowHeight, block.height);
  }

  const statusById = new Map(tasks.map((t) => [t.id, t.status]));
  const horizontal = rankdir === "LR";

  const nodes: TaskNodeType[] = tasks.map((task) => ({
    id: task.id,
    type: "task",
    data: { task },
    position: positions.get(task.id)!,
    // Explicit dimensions: edges render immediately, no measure pass.
    width: NODE_WIDTH,
    height: NODE_HEIGHT,
    sourcePosition: horizontal ? Position.Right : Position.Bottom,
    targetPosition: horizontal ? Position.Left : Position.Top,
    draggable: false,
    connectable: false,
  }));

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

/** One dagre layout for a connected component, normalized to its origin. */
function dagreBlock(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  rankdir: "TB" | "LR",
): Block {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir, nodesep: 24, ranksep: 48 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const task of tasks) g.setNode(task.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  for (const edge of edges) g.setEdge(edge.source, edge.target);
  dagre.layout(g);

  const positions = new Map<string, { x: number; y: number }>();
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const task of tasks) {
    const placed = g.node(task.id);
    // dagre positions node centers; React Flow wants top-left corners.
    const x = placed.x - NODE_WIDTH / 2;
    const y = placed.y - NODE_HEIGHT / 2;
    positions.set(task.id, { x, y });
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x + NODE_WIDTH);
    maxY = Math.max(maxY, y + NODE_HEIGHT);
  }
  for (const [id, p] of positions) positions.set(id, { x: p.x - minX, y: p.y - minY });
  return { positions, width: maxX - minX, height: maxY - minY };
}

/** Edge-less tasks as a near-square grid instead of one endless row. */
function gridBlock(tasks: readonly Task[]): Block {
  const columns = Math.ceil(Math.sqrt(tasks.length));
  const positions = new Map<string, { x: number; y: number }>();
  tasks.forEach((task, index) => {
    positions.set(task.id, {
      x: (index % columns) * (NODE_WIDTH + 24),
      y: Math.floor(index / columns) * (NODE_HEIGHT + 24),
    });
  });
  const rows = Math.ceil(tasks.length / columns);
  return {
    positions,
    width: columns * (NODE_WIDTH + 24) - 24,
    height: rows * (NODE_HEIGHT + 24) - 24,
  };
}
