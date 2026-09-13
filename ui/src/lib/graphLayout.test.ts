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

  it("collapses subtrees when expandedIds is an empty set", () => {
    const root = task({ title: "root" });
    const childA = task({ title: "child a" });
    const childB = task({ title: "child b" });
    const relations = [
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: childA.id }),
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: childB.id }),
    ];

    // Empty expandedIds = all collapsed: only root is visible.
    const { nodes, edges } = decompositionGraph([root, childA, childB], relations, new Set());

    expect(nodes).toHaveLength(1);
    expect(nodes[0].id).toBe(root.id);
    expect(nodes[0].data.childCount).toBe(2);
    expect(nodes[0].data.expanded).toBe(false);
    // No edges when children are hidden.
    expect(edges).toHaveLength(0);
  });

  it("expands only the nodes in expandedIds", () => {
    const root = task({ title: "root" });
    const epic1 = task({ title: "epic 1" });
    const epic2 = task({ title: "epic 2" });
    const leaf = task({ title: "leaf" });
    const relations = [
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic1.id }),
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic2.id }),
      relation({ type: "decomposition", source_task_id: epic1.id, target_task_id: leaf.id }),
    ];

    // Expand root only — epics appear, but leaf stays hidden.
    const { nodes } = decompositionGraph(
      [root, epic1, epic2, leaf],
      relations,
      new Set([root.id]),
    );

    const ids = new Set(nodes.map((n) => n.id));
    expect(ids).toContain(root.id);
    expect(ids).toContain(epic1.id);
    expect(ids).toContain(epic2.id);
    expect(ids).not.toContain(leaf.id);
    // epic1 knows it has a child but is collapsed.
    const epic1Node = nodes.find((n) => n.id === epic1.id)!;
    expect(epic1Node.data.childCount).toBe(1);
    expect(epic1Node.data.expanded).toBe(false);
  });

  it("provides childCount and expanded metadata without expandedIds (full expand)", () => {
    const parent = task({ title: "parent" });
    const child = task({ title: "child" });
    const relations = [
      relation({ type: "decomposition", source_task_id: parent.id, target_task_id: child.id }),
    ];

    // No expandedIds = show everything (backward compat).
    const { nodes } = decompositionGraph([parent, child], relations);

    const parentNode = nodes.find((n) => n.id === parent.id)!;
    expect(parentNode.data.childCount).toBe(1);
    expect(parentNode.data.expanded).toBe(true);
    const childNode = nodes.find((n) => n.id === child.id)!;
    expect(childNode.data.childCount).toBe(0);
    expect(childNode.data.expanded).toBe(false);
  });

  it("supports progressive expand through 4 levels (root → epic → sub-epic → task)", () => {
    const root = task({ title: "root" });
    const epic1 = task({ title: "epic 1" });
    const epic2 = task({ title: "epic 2" });
    const subA = task({ title: "sub-epic A" });
    const subB = task({ title: "sub-epic B" });
    const leafA1 = task({ title: "task A.1" });
    const leafA2 = task({ title: "task A.2" });
    const relations = [
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic1.id }),
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic2.id }),
      relation({ type: "decomposition", source_task_id: epic1.id, target_task_id: subA.id }),
      relation({ type: "decomposition", source_task_id: epic1.id, target_task_id: subB.id }),
      relation({ type: "decomposition", source_task_id: subA.id, target_task_id: leafA1.id }),
      relation({ type: "decomposition", source_task_id: subA.id, target_task_id: leafA2.id }),
    ];
    const all = [root, epic1, epic2, subA, subB, leafA1, leafA2];

    // Level 1: expand root only — epics visible, sub-epics hidden.
    const l1 = decompositionGraph(all, relations, new Set([root.id]));
    const l1Ids = new Set(l1.nodes.map((n) => n.id));
    expect(l1Ids).toEqual(new Set([root.id, epic1.id, epic2.id]));
    expect(l1.nodes.find((n) => n.id === epic1.id)!.data.childCount).toBe(2);

    // Level 2: expand root + epic1 — sub-epics visible, tasks hidden.
    const l2 = decompositionGraph(all, relations, new Set([root.id, epic1.id]));
    const l2Ids = new Set(l2.nodes.map((n) => n.id));
    expect(l2Ids).toEqual(new Set([root.id, epic1.id, epic2.id, subA.id, subB.id]));
    expect(l2.nodes.find((n) => n.id === subA.id)!.data.childCount).toBe(2);
    expect(l2.nodes.find((n) => n.id === subA.id)!.data.expanded).toBe(false);

    // Level 3: expand root + epic1 + subA — leaf tasks appear.
    const l3 = decompositionGraph(all, relations, new Set([root.id, epic1.id, subA.id]));
    const l3Ids = new Set(l3.nodes.map((n) => n.id));
    expect(l3Ids).toEqual(
      new Set([root.id, epic1.id, epic2.id, subA.id, subB.id, leafA1.id, leafA2.id]),
    );
    expect(l3.nodes.find((n) => n.id === leafA1.id)!.data.childCount).toBe(0);
    // Depth reads top → bottom at every level.
    const byId = new Map(l3.nodes.map((n) => [n.id, n]));
    expect(byId.get(epic1.id)!.position.y).toBeGreaterThan(byId.get(root.id)!.position.y);
    expect(byId.get(subA.id)!.position.y).toBeGreaterThan(byId.get(epic1.id)!.position.y);
    expect(byId.get(leafA1.id)!.position.y).toBeGreaterThan(byId.get(subA.id)!.position.y);
  });

  it("collapsed sub-epic hides its entire descendant tree, no stranding", () => {
    const root = task({ title: "root" });
    const epic = task({ title: "epic" });
    const sub = task({ title: "sub-epic" });
    const deep = task({ title: "deep task" });
    const relations = [
      relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic.id }),
      relation({ type: "decomposition", source_task_id: epic.id, target_task_id: sub.id }),
      relation({ type: "decomposition", source_task_id: sub.id, target_task_id: deep.id }),
    ];

    // Expand only root — epic visible, sub-epic and deep task hidden.
    const { nodes } = decompositionGraph(
      [root, epic, sub, deep],
      relations,
      new Set([root.id]),
    );
    const ids = new Set(nodes.map((n) => n.id));

    expect(ids).toEqual(new Set([root.id, epic.id]));
    // No stranded nodes: sub and deep are hidden, not surfaced as singletons.
    expect(ids).not.toContain(sub.id);
    expect(ids).not.toContain(deep.id);
    // epic knows it has 1 direct child.
    expect(nodes.find((n) => n.id === epic.id)!.data.childCount).toBe(1);
  });

  it("uses smoothstep edges", () => {
    const parent = task({ title: "parent" });
    const child = task({ title: "child" });
    const relations = [
      relation({ type: "decomposition", source_task_id: parent.id, target_task_id: child.id }),
    ];

    const { edges } = decompositionGraph([parent, child], relations);

    expect(edges[0].type).toBe("smoothstep");
  });
});

describe("dependencyGraph", () => {
  it("flips depends_on edges so prerequisites sit above dependents", () => {
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

    // TB layout: prerequisite above dependent.
    const byId = new Map(nodes.map((n) => [n.id, n]));
    expect(byId.get(dependent.id)!.position.y).toBeGreaterThan(
      byId.get(prerequisite.id)!.position.y,
    );
  });

  it("uses smoothstep edges with arrow markers", () => {
    const a = task({ title: "first" });
    const b = task({ title: "second" });
    const relations = [
      relation({ type: "depends_on", source_task_id: b.id, target_task_id: a.id }),
    ];

    const { edges } = dependencyGraph([a, b], relations);

    expect(edges[0].type).toBe("smoothstep");
    expect(edges[0].markerEnd).toBeDefined();
  });

  it("sets childCount to 0 and expanded to false for all nodes", () => {
    const a = task();
    const b = task();
    const relations = [
      relation({ type: "depends_on", source_task_id: b.id, target_task_id: a.id }),
    ];

    const { nodes } = dependencyGraph([a, b], relations);

    for (const node of nodes) {
      expect(node.data.childCount).toBe(0);
      expect(node.data.expanded).toBe(false);
    }
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

  it("grids edge-less tasks instead of one endless row", () => {
    const tasks = Array.from({ length: 9 }, (_, i) => task({ title: `t${i}` }));

    const { nodes } = dependencyGraph(tasks, []);

    const xs = new Set(nodes.map((n) => n.position.x));
    const ys = new Set(nodes.map((n) => n.position.y));
    // 9 singletons pack 3×3, not 9×1.
    expect(xs.size).toBe(3);
    expect(ys.size).toBe(3);
  });

  it("keeps a single-root mega-tree viewport-shaped by wrapping child rows", () => {
    // One root → 7 epics → 6 children each: a single component that a rank
    // layout would render as a ~11000px strip.
    const root = task({ title: "root" });
    const tasks = [root];
    const relations = [];
    for (let e = 0; e < 7; e++) {
      const epic = task({ title: `epic ${e}` });
      tasks.push(epic);
      relations.push(
        relation({ type: "decomposition", source_task_id: root.id, target_task_id: epic.id }),
      );
      for (let c = 0; c < 6; c++) {
        const child = task({ title: `child ${e}.${c}` });
        tasks.push(child);
        relations.push(
          relation({ type: "decomposition", source_task_id: epic.id, target_task_id: child.id }),
        );
      }
    }

    const { nodes } = decompositionGraph(tasks, relations);

    const byId = new Map(nodes.map((n) => [n.id, n]));
    const maxX = Math.max(...nodes.map((n) => n.position.x + NODE_WIDTH));
    expect(maxX).toBeLessThanOrEqual(2400);
    // Root centered above its child area, not pinned to the left edge.
    const rootNode = byId.get(root.id)!;
    expect(rootNode.position.y).toBe(0);
    expect(rootNode.position.x).toBeGreaterThan(0);
    // Depth still reads top → bottom.
    for (const rel of relations) {
      expect(byId.get(rel.target_task_id)!.position.y).toBeGreaterThan(
        byId.get(rel.source_task_id)!.position.y,
      );
    }
  });

  it("wraps disconnected components into rows, bounding the canvas width", () => {
    // 8 parent→(3 children) components: side-by-side they'd be ~8000px wide.
    const tasks = [];
    const relations = [];
    for (let i = 0; i < 8; i++) {
      const parent = task({ title: `epic ${i}` });
      tasks.push(parent);
      for (let c = 0; c < 3; c++) {
        const child = task({ title: `child ${i}.${c}` });
        tasks.push(child);
        relations.push(
          relation({ type: "decomposition", source_task_id: parent.id, target_task_id: child.id }),
        );
      }
    }

    const { nodes } = decompositionGraph(tasks, relations);

    const maxX = Math.max(...nodes.map((n) => n.position.x + NODE_WIDTH));
    const maxY = Math.max(...nodes.map((n) => n.position.y + NODE_HEIGHT));
    expect(maxX).toBeLessThanOrEqual(2400);
    // More than one shelf row means the packing actually wrapped.
    expect(maxY).toBeGreaterThan(NODE_HEIGHT * 4);
  });
});
