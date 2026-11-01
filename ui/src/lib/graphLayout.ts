import { graphlib, layout } from "@dagrejs/dagre";
import { MarkerType } from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import type { GraphNodesItem } from "../api/generated/model/graphNodesItem";
import type { Dependency } from "../api/generated/model/dependency";
import type { Graph } from "../api/generated/model/graph";

export const NODE_WIDTH = 280;
export const NODE_HEIGHT = 60;

const GROUP_PADDING = 40;
const GROUP_HEADER = 52;
const NODESEP = 40;
const RANKSEP = 60;
const GROUP_GAP = 60;

export interface GoalCluster {
  goalId: string;
  goalTitle: string;
  goalCompleted: boolean;
  epicCounts: { total: number; done: number; cancelled: number; waived: number };
  graph: Graph;
}

export function computeLayout(
  nodes: GraphNodesItem[],
  dependencies: Dependency[],
): { nodes: Node[]; edges: Edge[] } {
  if (nodes.length === 0) return { nodes: [], edges: [] };

  const g = new graphlib.Graph();
  g.setGraph({ rankdir: "TB", nodesep: NODESEP, ranksep: RANKSEP });
  g.setDefaultEdgeLabel(() => ({}));

  for (const n of nodes) {
    g.setNode(n.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  }
  for (const d of dependencies) {
    g.setEdge(d.prerequisite_id, d.dependent_id);
  }

  layout(g);

  const rfNodes: Node[] = nodes.map((n) => {
    const pos = g.node(n.id) as { x: number; y: number };
    return {
      id: n.id,
      type: "graphNode",
      position: { x: pos.x - NODE_WIDTH / 2, y: pos.y - NODE_HEIGHT / 2 },
      data: n,
    };
  });

  const rfEdges = dependencies.map<Edge>((d) => ({
    id: d.id,
    source: d.prerequisite_id,
    target: d.dependent_id,
    markerEnd: { type: MarkerType.ArrowClosed },
    data: d as unknown as Record<string, unknown>,
  }));

  return { nodes: rfNodes, edges: rfEdges };
}

export function computeProjectLayout(clusters: GoalCluster[]): {
  nodes: Node[];
  edges: Edge[];
} {
  if (clusters.length === 0) return { nodes: [], edges: [] };

  const allNodes: Node[] = [];
  const allEdges: Edge[] = [];

  const columns = Math.min(clusters.length, 3);
  let col = 0;
  let row = 0;
  const colWidths: number[] = Array.from({ length: columns }, () => 0);
  const rowHeights: number[] = [];

  interface ClusterLayout {
    goalId: string;
    width: number;
    height: number;
    nodes: Node[];
    edges: Edge[];
    col: number;
    row: number;
    cluster: GoalCluster;
  }

  const layouts: ClusterLayout[] = [];

  for (const cluster of clusters) {
    const { graph, goalId } = cluster;
    const nodes = graph.nodes;
    const deps = graph.dependencies;

    let clusterNodes: Node[] = [];
    let clusterEdges: Edge[] = [];
    let clusterWidth = NODE_WIDTH + GROUP_PADDING * 2;
    let clusterHeight = GROUP_HEADER + NODE_HEIGHT + GROUP_PADDING;

    if (nodes.length > 0) {
      const g = new graphlib.Graph();
      g.setGraph({ rankdir: "TB", nodesep: NODESEP, ranksep: RANKSEP });
      g.setDefaultEdgeLabel(() => ({}));

      for (const n of nodes) {
        g.setNode(n.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
      }
      for (const d of deps) {
        g.setEdge(d.prerequisite_id, d.dependent_id);
      }

      layout(g);

      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;

      for (const n of nodes) {
        const pos = g.node(n.id) as { x: number; y: number };
        const x = pos.x - NODE_WIDTH / 2;
        const y = pos.y - NODE_HEIGHT / 2;
        minX = Math.min(minX, x);
        minY = Math.min(minY, y);
        maxX = Math.max(maxX, x + NODE_WIDTH);
        maxY = Math.max(maxY, y + NODE_HEIGHT);
      }

      clusterNodes = nodes.map((n) => {
        const pos = g.node(n.id) as { x: number; y: number };
        return {
          id: n.id,
          type: "graphNode",
          parentId: goalId,
          extent: "parent" as const,
          position: {
            x: pos.x - NODE_WIDTH / 2 - minX + GROUP_PADDING,
            y: pos.y - NODE_HEIGHT / 2 - minY + GROUP_HEADER,
          },
          data: n,
        };
      });

      clusterEdges = deps.map<Edge>((d) => ({
        id: d.id,
        source: d.prerequisite_id,
        target: d.dependent_id,
        markerEnd: { type: MarkerType.ArrowClosed },
        data: d as unknown as Record<string, unknown>,
      }));

      clusterWidth = maxX - minX + GROUP_PADDING * 2;
      clusterHeight = maxY - minY + GROUP_HEADER + GROUP_PADDING;
    }

    colWidths[col] = Math.max(colWidths[col], clusterWidth);
    if (!rowHeights[row]) rowHeights[row] = 0;
    rowHeights[row] = Math.max(rowHeights[row], clusterHeight);

    layouts.push({
      goalId,
      width: clusterWidth,
      height: clusterHeight,
      nodes: clusterNodes,
      edges: clusterEdges,
      col,
      row,
      cluster,
    });

    col++;
    if (col >= columns) {
      col = 0;
      row++;
    }
  }

  for (const cl of layouts) {
    let offsetX = 0;
    for (let c = 0; c < cl.col; c++) {
      offsetX += colWidths[c] + GROUP_GAP;
    }
    let offsetY = 0;
    for (let r = 0; r < cl.row; r++) {
      offsetY += rowHeights[r] + GROUP_GAP;
    }

    allNodes.push({
      id: cl.goalId,
      type: "goalGroup",
      position: { x: offsetX, y: offsetY },
      data: {
        title: cl.cluster.goalTitle,
        completed: cl.cluster.goalCompleted,
        epicCounts: cl.cluster.epicCounts,
      },
      style: { width: cl.width, height: cl.height },
    });

    for (const n of cl.nodes) {
      allNodes.push(n);
    }
    for (const e of cl.edges) {
      allEdges.push(e);
    }
  }

  return { nodes: allNodes, edges: allEdges };
}

export function classifyDagPosition(
  nodeId: string,
  dependencies: Dependency[],
): { root: boolean; leaf: boolean } {
  let hasIncoming = false;
  let hasOutgoing = false;
  for (const d of dependencies) {
    if (d.dependent_id === nodeId) hasIncoming = true;
    if (d.prerequisite_id === nodeId) hasOutgoing = true;
    if (hasIncoming && hasOutgoing) break;
  }
  return { root: !hasIncoming, leaf: !hasOutgoing };
}

export const DAG_START_ID = "__dag_start__";
export const DAG_END_ID = "__dag_end__";
const ENDPOINT_WIDTH = 120;
const ENDPOINT_HEIGHT = 50;

export interface DagEndpoint {
  id: string;
  label: string;
}

export function addDagEndpoints(nodes: Node[], edges: Edge[]): { nodes: Node[]; edges: Edge[] } {
  if (nodes.length === 0) return { nodes, edges };

  const hasIncoming = new Set<string>();
  const hasOutgoing = new Set<string>();
  for (const e of edges) {
    hasIncoming.add(e.target);
    hasOutgoing.add(e.source);
  }

  const roots = nodes.filter((n) => !hasIncoming.has(n.id));
  const leaves = nodes.filter((n) => !hasOutgoing.has(n.id));

  const startNode: Node = {
    id: DAG_START_ID,
    type: "dagEndpoint",
    position: { x: 0, y: 0 },
    data: { label: "start" },
  };

  const endNode: Node = {
    id: DAG_END_ID,
    type: "dagEndpoint",
    position: { x: 0, y: 0 },
    data: { label: "end" },
  };

  const startEdges: Edge[] = roots.map((n) => ({
    id: `__start_${n.id}`,
    source: DAG_START_ID,
    target: n.id,
    markerEnd: { type: MarkerType.ArrowClosed },
  }));

  const endEdges: Edge[] = leaves.map((n) => ({
    id: `__end_${n.id}`,
    source: n.id,
    target: DAG_END_ID,
    markerEnd: { type: MarkerType.ArrowClosed },
  }));

  return {
    nodes: [startNode, ...nodes, endNode],
    edges: [...startEdges, ...edges, ...endEdges],
  };
}

function nodeWidth(n: Node): number {
  if (n.type === "dagEndpoint") return ENDPOINT_WIDTH;
  const sw = n.style?.width;
  if (typeof sw === "number") return sw;
  return NODE_WIDTH;
}

function nodeHeight(n: Node): number {
  if (n.type === "dagEndpoint") return ENDPOINT_HEIGHT;
  const sh = n.style?.height;
  if (typeof sh === "number") return sh;
  return NODE_HEIGHT;
}

export function computeLayoutWithEndpoints(
  nodes: Node[],
  edges: Edge[],
): { nodes: Node[]; edges: Edge[] } {
  if (nodes.length === 0) return { nodes: [], edges: [] };

  const g = new graphlib.Graph();
  g.setGraph({ rankdir: "TB", nodesep: NODESEP, ranksep: RANKSEP });
  g.setDefaultEdgeLabel(() => ({}));

  for (const n of nodes) {
    const w = nodeWidth(n);
    const h = nodeHeight(n);
    g.setNode(n.id, { width: w, height: h });
  }
  for (const e of edges) {
    g.setEdge(e.source, e.target);
  }

  layout(g);

  const positioned = nodes.map((n) => {
    const pos = g.node(n.id) as { x: number; y: number };
    const w = nodeWidth(n);
    const h = nodeHeight(n);
    return { ...n, position: { x: pos.x - w / 2, y: pos.y - h / 2 } };
  });

  return { nodes: positioned, edges };
}

export function topologyKey(nodes: GraphNodesItem[], dependencies: Dependency[]): string {
  const sortedIds = nodes.map((n) => n.id).toSorted();
  const sortedDeps = dependencies.map((d) => `${d.prerequisite_id}>${d.dependent_id}`).toSorted();
  return `${sortedIds.join(",")}|${sortedDeps.join(",")}`;
}
