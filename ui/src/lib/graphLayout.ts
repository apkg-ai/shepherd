import { graphlib, layout } from "@dagrejs/dagre";
import { MarkerType } from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import type { GraphNodesItem } from "../api/generated/model/graphNodesItem";
import type { Dependency } from "../api/generated/model/dependency";

export const NODE_WIDTH = 200;
export const NODE_HEIGHT = 80;

const NODESEP = 50;
const RANKSEP = 80;

export function computeLayout(
  nodes: GraphNodesItem[],
  dependencies: Dependency[],
): { nodes: Node[]; edges: Edge[] } {
  if (nodes.length === 0) return { nodes: [], edges: [] };

  const g = new graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: NODESEP, ranksep: RANKSEP });
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

export function topologyKey(
  nodes: GraphNodesItem[],
  dependencies: Dependency[],
): string {
  const sortedIds = nodes.map((n) => n.id).toSorted();
  const sortedDeps = dependencies
    .map((d) => `${d.prerequisite_id}>${d.dependent_id}`)
    .toSorted();
  return `${sortedIds.join(",")}|${sortedDeps.join(",")}`;
}
