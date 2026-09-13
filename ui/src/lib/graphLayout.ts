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
 *
 * The tree lens supports progressive disclosure: when `expandedIds` is
 * provided, only nodes whose parents are in the set (or roots) are laid out.
 * Each node carries a `childCount` so the UI can render an expand chip.
 */

export const NODE_WIDTH = 248;
export const NODE_HEIGHT = 76;

export interface TaskNodeData {
  task: Task;
  /** Direct children in the decomposition tree. 0 in the flow lens. */
  childCount: number;
  /** Whether this node's children are visible. Always false in the flow lens. */
  expanded: boolean;
  /** Set by the graph screen when neighbor fade is active (not a layout concern). */
  faded?: boolean;
}

export type TaskNodeType = Node<TaskNodeData, "task">;

/** Virtual Start/End nodes anchoring the epic flow. */
export const BOUNDARY_SIZE = 64;
export type BoundaryNodeType = Node<{ label: string }, "boundary">;

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

/**
 * Decomposition lens: parent above child, top→bottom.
 *
 * When `expandedIds` is provided, only nodes whose parents are in the set
 * (or roots with no parent) appear. Pass `undefined` to show everything
 * (backward compat / tests), or an empty Set to collapse all subtrees.
 */
export function decompositionGraph(
  tasks: readonly Task[],
  relations: readonly Relation[],
  expandedIds?: ReadonlySet<string>,
): GraphModel {
  return treeLayout(tasks, lensEdges(tasks, relations, "decomposition"), expandedIds);
}

/**
 * Dependency lens: prerequisite before dependent, start→end left→right.
 *
 * When `nodeMeta` is provided, each node carries the given `childCount`
 * and `expanded` values (used by the epic-only flow to show subtask chips).
 */
export function dependencyGraph(
  tasks: readonly Task[],
  relations: readonly Relation[],
  nodeMeta?: ReadonlyMap<string, { childCount: number; expanded: boolean }>,
): GraphModel {
  // Flip: depends_on points dependent → prerequisite, the lens reads
  // prerequisite → dependent.
  const edges = lensEdges(tasks, relations, "depends_on").map((e) => ({
    relation: e.relation,
    source: e.relation.target_task_id,
    target: e.relation.source_task_id,
  }));
  return layout(tasks, edges, "TB", { markerEnd: { type: MarkerType.ArrowClosed } }, nodeMeta);
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
 * Flow-lens layout. Dagre lays disconnected components side-by-side in one
 * endless rank, so: dagre per connected component, edge-less tasks gathered
 * into one compact grid, then the blocks shelf-packed into rows — the canvas
 * stays near-viewport-shaped at any project size.
 */
function layout(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  rankdir: "TB" | "LR",
  edgeExtras: Partial<Edge> = {},
  nodeMeta?: ReadonlyMap<string, NodeMeta>,
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

  return assemble(tasks, edges, packBlocks(blocks, MAX_ROW_WIDTH), rankdir === "LR", edgeExtras, nodeMeta);
}

/** Per-node metadata produced during tree layout for the UI expand chip. */
interface NodeMeta {
  childCount: number;
  expanded: boolean;
}

/**
 * Decomposition-only layout: a recursive tidy tree where a parent's child
 * blocks WRAP into rows instead of forming one endless rank — dagre can't do
 * this, and it's what keeps a shallow-wide tree (one root, many epics, many
 * leaves) viewport-shaped. Parents sit centered above their child area.
 *
 * When `expandedIds` is provided, only children of expanded nodes are laid
 * out. Pass `undefined` to show everything (tests / no progressive disclosure).
 */
function treeLayout(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  expandedIds?: ReadonlySet<string>,
): GraphModel {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const childrenOf = new Map<string, string[]>();
  const hasParent = new Set<string>();
  for (const edge of edges) {
    // Decomposition orientation: source is the parent, target the child.
    const siblings = childrenOf.get(edge.source);
    if (siblings === undefined) childrenOf.set(edge.source, [edge.target]);
    else siblings.push(edge.target);
    hasParent.add(edge.target);
  }

  const nodeMeta = new Map<string, NodeMeta>();
  const visited = new Set<string>();

  const subtree = (id: string): Block => {
    visited.add(id);
    const kids = (childrenOf.get(id) ?? []).filter((k) => !visited.has(k) && byId.has(k));
    const childCount = kids.length;
    const isExpanded = childCount > 0 && (expandedIds === undefined || expandedIds.has(id));
    nodeMeta.set(id, { childCount, expanded: isExpanded });

    // Collapsed or leaf: render just this node.
    if (!isExpanded) {
      // Mark all descendants as visited so the stranded-node check below
      // doesn't accidentally surface them as singletons.
      const markHidden = (ids: readonly string[]) => {
        for (const kid of ids) {
          if (visited.has(kid)) continue;
          visited.add(kid);
          markHidden((childrenOf.get(kid) ?? []).filter((k) => byId.has(k)));
        }
      };
      markHidden(kids);
      return {
        positions: new Map([[id, { x: 0, y: 0 }]]),
        width: NODE_WIDTH,
        height: NODE_HEIGHT,
      };
    }

    const kidBlocks = kids.map(subtree);
    const childArea = packBlocks(kidBlocks, MAX_ROW_WIDTH);
    let width = NODE_WIDTH;
    let height = 0;
    for (const p of childArea.values()) {
      width = Math.max(width, p.x + NODE_WIDTH);
      height = Math.max(height, p.y + NODE_HEIGHT);
    }
    const positions = new Map<string, { x: number; y: number }>();
    // Parent centered over its children, children one level down.
    positions.set(id, { x: (width - NODE_WIDTH) / 2, y: 0 });
    for (const [childId, p] of childArea) {
      positions.set(childId, { x: p.x, y: p.y + NODE_HEIGHT + 48 });
    }
    return { positions, width, height: height + NODE_HEIGHT + 48 };
  };

  const blocks: Block[] = [];
  const singles: Task[] = [];
  for (const task of tasks) {
    if (hasParent.has(task.id)) continue;
    if (childrenOf.has(task.id)) blocks.push(subtree(task.id));
    else {
      nodeMeta.set(task.id, { childCount: 0, expanded: false });
      singles.push(task);
    }
  }
  // Defensive: anything unreachable (malformed parent cycles) still renders.
  const stranded = tasks.filter((t) => !visited.has(t.id) && !singles.includes(t));
  if (stranded.length > 0) {
    for (const t of stranded) nodeMeta.set(t.id, { childCount: 0, expanded: false });
    singles.push(...stranded);
  }
  if (singles.length > 0) blocks.push(gridBlock(singles));

  const positions = packBlocks(blocks, MAX_ROW_WIDTH);

  // Only keep edges whose both endpoints are visible (collapsed children are hidden).
  const visibleEdges = edges.filter((e) => positions.has(e.source) && positions.has(e.target));

  return assemble(tasks, visibleEdges, positions, false, {}, nodeMeta);
}

/** Shelf-packs blocks into rows capped at `maxWidth`; returns merged positions. */
function packBlocks(
  blocks: readonly Block[],
  maxWidth: number,
): Map<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  let x = 0;
  let y = 0;
  let rowHeight = 0;
  for (const block of blocks) {
    if (x > 0 && x + block.width > maxWidth) {
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
  return positions;
}

/** Positions + lens edges → React Flow nodes/edges. */
function assemble(
  tasks: readonly Task[],
  edges: readonly LensEdge[],
  positions: ReadonlyMap<string, { x: number; y: number }>,
  horizontal: boolean,
  edgeExtras: Partial<Edge> = {},
  nodeMeta?: ReadonlyMap<string, NodeMeta>,
): GraphModel {
  const statusById = new Map(tasks.map((t) => [t.id, t.status]));

  // Only create nodes for tasks that have a position (collapsed children are excluded).
  const visibleTasks = tasks.filter((t) => positions.has(t.id));

  const nodes: TaskNodeType[] = visibleTasks.map((task) => ({
    id: task.id,
    type: "task",
    data: {
      task,
      childCount: nodeMeta?.get(task.id)?.childCount ?? 0,
      expanded: nodeMeta?.get(task.id)?.expanded ?? false,
    },
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
    type: "smoothstep",
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
