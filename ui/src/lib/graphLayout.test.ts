import { describe, expect, it } from "vitest";
import { MarkerType } from "@xyflow/react";
import {
  computeLayout,
  topologyKey,
  classifyDagPosition,
  computeProjectLayout,
} from "./graphLayout";
import type { GoalCluster } from "./graphLayout";
import type { GraphNodesItem } from "../api/generated/model/graphNodesItem";
import type { Dependency } from "../api/generated/model/dependency";

const node = (id: string, title = "Node"): GraphNodesItem => ({
  id,
  title,
  status: "open",
  eligibility: {
    can_plan: true,
    can_execute: false,
    can_review: false,
    reasons: [],
    allowed_actions: [],
  },
  counts: { total: 0, done: 0, cancelled: 0, waived: 0 },
});

const dep = (id: string, prerequisite_id: string, dependent_id: string): Dependency => ({
  id,
  revision: 1,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  project_id: "proj-1",
  level: "epic",
  dependent_id,
  prerequisite_id,
});

describe("computeLayout", () => {
  it("returns empty arrays for zero nodes", () => {
    const result = computeLayout([], []);
    expect(result).toEqual({ nodes: [], edges: [] });
  });

  it("assigns positions to nodes", () => {
    const result = computeLayout([node("a"), node("b")], [dep("d1", "a", "b")]);
    expect(result.nodes).toHaveLength(2);
    for (const n of result.nodes) {
      expect(n.position.x).toEqual(expect.any(Number));
      expect(n.position.y).toEqual(expect.any(Number));
    }
    expect(result.edges).toHaveLength(1);
    expect(result.edges[0].source).toBe("a");
    expect(result.edges[0].target).toBe("b");
  });

  it("creates top-to-bottom layout", () => {
    const result = computeLayout([node("a"), node("b")], [dep("d1", "a", "b")]);
    const prerequisite = result.nodes.find((n) => n.id === "a")!;
    const dependent = result.nodes.find((n) => n.id === "b")!;
    expect(prerequisite.position.y).toBeLessThan(dependent.position.y);
  });

  it("handles disconnected nodes", () => {
    const result = computeLayout([node("a"), node("b"), node("c")], []);
    expect(result.nodes).toHaveLength(3);
    for (const n of result.nodes) {
      expect(Number.isFinite(n.position.x)).toBe(true);
      expect(Number.isFinite(n.position.y)).toBe(true);
    }
  });

  it("passes node data through", () => {
    const result = computeLayout([node("a", "Alpha")], []);
    const data = result.nodes[0].data as GraphNodesItem;
    expect(data.id).toBe("a");
    expect(data.title).toBe("Alpha");
    expect(data.status).toBe("open");
  });

  it("creates edges with arrowhead markers", () => {
    const result = computeLayout([node("a"), node("b")], [dep("d1", "a", "b")]);
    expect(result.edges[0].markerEnd).toEqual({
      type: MarkerType.ArrowClosed,
    });
  });
});

describe("topologyKey", () => {
  it("is stable regardless of input order", () => {
    const nodes = [node("b"), node("a"), node("c")];
    const deps = [dep("d2", "b", "c"), dep("d1", "a", "b")];

    const nodesReordered = [node("c"), node("a"), node("b")];
    const depsReordered = [dep("d1", "a", "b"), dep("d2", "b", "c")];

    expect(topologyKey(nodes, deps)).toBe(topologyKey(nodesReordered, depsReordered));
  });

  it("differs when node added", () => {
    const base = topologyKey([node("a"), node("b")], []);
    const extended = topologyKey([node("a"), node("b"), node("c")], []);
    expect(base).not.toBe(extended);
  });

  it("differs when dependency added", () => {
    const nodes = [node("a"), node("b")];
    const base = topologyKey(nodes, []);
    const extended = topologyKey(nodes, [dep("d1", "a", "b")]);
    expect(base).not.toBe(extended);
  });

  it("is same when only status changes", () => {
    const openNodes = [node("a"), node("b")];
    const doneNodes: GraphNodesItem[] = [
      { ...node("a"), status: "done" },
      { ...node("b"), status: "done" },
    ];
    const deps = [dep("d1", "a", "b")];
    expect(topologyKey(openNodes, deps)).toBe(topologyKey(doneNodes, deps));
  });
});

describe("classifyDagPosition", () => {
  it("root has no incoming edges", () => {
    const deps = [dep("d1", "a", "b")];
    expect(classifyDagPosition("a", deps)).toEqual({ root: true, leaf: false });
  });

  it("leaf has no outgoing edges", () => {
    const deps = [dep("d1", "a", "b")];
    expect(classifyDagPosition("b", deps)).toEqual({ root: false, leaf: true });
  });

  it("middle node is neither root nor leaf", () => {
    const deps = [dep("d1", "a", "b"), dep("d2", "b", "c")];
    expect(classifyDagPosition("b", deps)).toEqual({ root: false, leaf: false });
  });

  it("isolated node is both root and leaf", () => {
    expect(classifyDagPosition("x", [])).toEqual({ root: true, leaf: true });
  });
});

describe("computeProjectLayout", () => {
  it("returns empty for no clusters", () => {
    expect(computeProjectLayout([])).toEqual({ nodes: [], edges: [] });
  });

  it("creates goal group nodes with epic children", () => {
    const cluster: GoalCluster = {
      goalId: "g1",
      goalTitle: "Goal One",
      goalCompleted: false,
      epicCounts: { total: 2, done: 0, cancelled: 0, waived: 0 },
      graph: {
        level: "epic",
        scope_id: "g1",
        nodes: [node("e1"), node("e2")],
        dependencies: [dep("d1", "e1", "e2")],
      },
    };
    const result = computeProjectLayout([cluster]);

    const groupNode = result.nodes.find((n) => n.id === "g1");
    expect(groupNode).toBeDefined();
    expect(groupNode!.type).toBe("goalGroup");

    const epicNodes = result.nodes.filter((n) => n.type === "graphNode");
    expect(epicNodes).toHaveLength(2);
    expect(epicNodes[0].parentId).toBe("g1");

    expect(result.edges).toHaveLength(1);
  });

  it("arranges multiple goals in a grid", () => {
    const clusters: GoalCluster[] = [
      {
        goalId: "g1",
        goalTitle: "Goal A",
        goalCompleted: false,
        epicCounts: { total: 1, done: 0, cancelled: 0, waived: 0 },
        graph: { level: "epic", scope_id: "g1", nodes: [node("e1")], dependencies: [] },
      },
      {
        goalId: "g2",
        goalTitle: "Goal B",
        goalCompleted: true,
        epicCounts: { total: 1, done: 1, cancelled: 0, waived: 0 },
        graph: { level: "epic", scope_id: "g2", nodes: [node("e2")], dependencies: [] },
      },
    ];
    const result = computeProjectLayout(clusters);

    const groups = result.nodes.filter((n) => n.type === "goalGroup");
    expect(groups).toHaveLength(2);
    expect(groups[0].position.x).not.toBe(groups[1].position.x);
  });
});
