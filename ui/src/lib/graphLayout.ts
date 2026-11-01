import { graphlib, layout } from "@dagrejs/dagre";
import { MarkerType } from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import type { GraphNodesItem } from "../api/generated/model/graphNodesItem";
import type { Dependency } from "../api/generated/model/dependency";

export const NODE_WIDTH = 280;
export const NODE_HEIGHT = 60;

const NODESEP = 40;
const RANKSEP = 60;

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

export const DAG_START_ID = "__dag_start__";
export const DAG_END_ID = "__dag_end__";
const ENDPOINT_WIDTH = 120;
const ENDPOINT_HEIGHT = 50;

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
