import { describe, expect, it } from "vitest";
import { relation, task } from "../test/fixtures";
import { NODE_HEIGHT, NODE_WIDTH, decompositionGraph, dependencyGraph } from "./graphLayout";

describe("decompositionGraph", () => {
  it("lays children below their parent", () => {
    const parent = task({ title: "parent" });
    const childA = task({ title: "child a" });
    const childB = task({ title: "child b" });
    const relations = [
      relation({ type: "decomposition", source_task_id: parent.id, target_task_id: childA.id }),
      relation({ type: "decomposition", source_task_id: parent.id, target_task_id: childB.id }),
    ];

    const { nodes, edges } = decompositionGraph([parent, childA, childB], relations);

    const byId = new Map(nodes.map((n) => [n.id, n]));
    expect(nodes).toHaveLength(3);
    expect(edges).toHaveLength(2);
    for (const child of [childA, childB]) {
      expect(byId.get(child.id)!.position.y).toBeGreaterThan(byId.get(parent.id)!.position.y);
    }
    // Edges keep the relation's own direction: parent → child.
    expect(edges.map((e) => e.source)).toEqual([parent.id, parent.id]);
  });

  it("ignores depends_on relations", () => {
    const a = task();
    const b = task();
    const relations = [
      relation({ type: "depends_on", source_task_id: a.id, target_task_id: b.id }),
    ];

    const { edges } = decompositionGraph([a, b], relations);

    expect(edges).toHaveLength(0);
  });
});

describe("dependencyGraph", () => {
  it("flips depends_on edges so prerequisites sit left of dependents", () => {
    const prerequisite = task({ title: "first" });
    const dependent = task({ title: "second" });
    // dependent depends_on prerequisite.
    const relations = [
      relation({
        type: "depends_on",
        source_task_id: dependent.id,
        target_task_id: prerequisite.id,
      }),
    ];

    const { nodes, edges } = dependencyGraph([prerequisite, dependent], relations);

    expect(edges).toHaveLength(1);
    expect(edges[0].source).toBe(prerequisite.id);
    expect(edges[0].target).toBe(dependent.id);
    expect(edges[0].markerEnd).toBeDefined();

    const byId = new Map(nodes.map((n) => [n.id, n]));
    expect(byId.get(dependent.id)!.position.x).toBeGreaterThan(
      byId.get(prerequisite.id)!.position.x,
    );
  });

  it("animates edges touching an in_progress task", () => {
    const doing = task({ status: "in_progress" });
    const next = task({ status: "approved" });
    const other = task({ status: "done" });
    const third = task({ status: "ready" });
    const relations = [
      relation({ type: "depends_on", source_task_id: next.id, target_task_id: doing.id }),
      relation({ type: "depends_on", source_task_id: third.id, target_task_id: other.id }),
    ];

    const { edges } = dependencyGraph([doing, next, other, third], relations);

    const byRelation = new Map(edges.map((e) => [e.id, e]));
    expect(byRelation.get(relations[0].id)!.animated).toBe(true);
    expect(byRelation.get(relations[1].id)!.animated).toBe(false);
  });
});

describe("both lenses", () => {
  it("places isolated tasks and maps ids", () => {
    const lonely = task({ title: "isolated" });

    for (const build of [decompositionGraph, dependencyGraph]) {
      const { nodes, edges } = build([lonely], []);
      expect(edges).toHaveLength(0);
      expect(nodes).toHaveLength(1);
      expect(nodes[0].id).toBe(lonely.id);
      expect(nodes[0].type).toBe("task");
      expect(nodes[0].data.task).toBe(lonely);
      expect(Number.isFinite(nodes[0].position.x)).toBe(true);
      expect(Number.isFinite(nodes[0].position.y)).toBe(true);
    }
  });

  it("drops self-edges and edges with unloaded endpoints", () => {
    const a = task();
    const b = task();
    const unloaded = task();
    const relations = [
      relation({ type: "depends_on", source_task_id: a.id, target_task_id: a.id }),
      relation({ type: "depends_on", source_task_id: b.id, target_task_id: unloaded.id }),
      relation({ type: "decomposition", source_task_id: a.id, target_task_id: unloaded.id }),
    ];

    expect(dependencyGraph([a, b], relations).edges).toHaveLength(0);
    expect(decompositionGraph([a, b], relations).edges).toHaveLength(0);
  });

  it("keeps edge ids stable as relation ids across lenses", () => {
    const parent = task();
    const child = task();
    const rels = [
      relation({ type: "decomposition", source_task_id: parent.id, target_task_id: child.id }),
      relation({ type: "depends_on", source_task_id: child.id, target_task_id: parent.id }),
    ];

    expect(decompositionGraph([parent, child], rels).edges[0].id).toBe(rels[0].id);
    expect(dependencyGraph([parent, child], rels).edges[0].id).toBe(rels[1].id);
  });

  it("positions are top-left corners offset from dagre centers", () => {
    const only = task();

    const { nodes } = decompositionGraph([only], []);

    // A single node sits at the layout origin: center (w/2, h/2) → corner (0, 0).
    expect(nodes[0].position).toEqual({ x: 0, y: 0 });
    expect(NODE_WIDTH).toBeGreaterThan(0);
    expect(NODE_HEIGHT).toBeGreaterThan(0);
  });
});
