import { describe, expect, it } from "vitest";
import type { Relation, TaskStatus } from "../api/generated/model";
import { relation, task } from "../test/fixtures";
import { buildTaskTree, describeDependencies, type RelationsByTask } from "./taskTree";

function relMap(entries: [string, Relation[] | undefined][]): RelationsByTask {
  return new Map(entries);
}

describe("buildTaskTree", () => {
  it("groups loaded children under their parent in page order", () => {
    const parent = task({ title: "Parent" });
    const childA = task({ title: "A" });
    const childB = task({ title: "B" });
    const edgeA = relation({
      type: "decomposition",
      source_task_id: parent.id,
      target_task_id: childA.id,
    });
    const edgeB = relation({
      type: "decomposition",
      source_task_id: parent.id,
      target_task_id: childB.id,
    });
    const roots = buildTaskTree(
      [parent, childA, childB],
      relMap([
        [parent.id, [edgeA, edgeB]],
        [childA.id, [edgeA]],
        [childB.id, [edgeB]],
      ]),
    );
    expect(roots).toHaveLength(1);
    expect(roots[0]!.task.id).toBe(parent.id);
    expect(roots[0]!.subtaskCount).toBe(2);
    expect(roots[0]!.children.map((c) => c.task.title)).toEqual(["A", "B"]);
    expect(roots[0]!.children[0]!.parentInView).toBe(true);
  });

  it("keeps tasks with unloaded parents top-level, flagged as out of view", () => {
    const orphan = task({ title: "Orphan" });
    const edge = relation({
      type: "decomposition",
      source_task_id: "not-loaded",
      target_task_id: orphan.id,
    });
    const roots = buildTaskTree([orphan], relMap([[orphan.id, [edge]]]));
    expect(roots).toHaveLength(1);
    expect(roots[0]!.parentId).toBe("not-loaded");
    expect(roots[0]!.parentInView).toBe(false);
  });

  it("treats tasks with unloaded relations as top-level without counts", () => {
    const pending = task({ title: "Loading" });
    const roots = buildTaskTree([pending], relMap([[pending.id, undefined]]));
    expect(roots[0]!.subtaskCount).toBe(0);
    expect(roots[0]!.parentId).toBeNull();
    expect(roots[0]!.dependsOnIds).toEqual([]);
  });

  it("collects depends_on prerequisites", () => {
    const dependent = task({ title: "Dependent" });
    const dep1 = task({ title: "Dep 1" });
    const edges = [
      relation({
        type: "depends_on",
        source_task_id: dependent.id,
        target_task_id: dep1.id,
      }),
      relation({
        type: "depends_on",
        source_task_id: "someone-else",
        target_task_id: dependent.id,
      }),
    ];
    const roots = buildTaskTree([dependent], relMap([[dependent.id, edges]]));
    expect(roots[0]!.dependsOnIds).toEqual([dep1.id]);
  });

  it("guards against decomposition cycles and self-parents", () => {
    const a = task({ title: "A" });
    const b = task({ title: "B" });
    const aParentOfB = relation({
      type: "decomposition",
      source_task_id: a.id,
      target_task_id: b.id,
    });
    const bParentOfA = relation({
      type: "decomposition",
      source_task_id: b.id,
      target_task_id: a.id,
    });
    const selfEdge = relation({
      type: "decomposition",
      source_task_id: a.id,
      target_task_id: a.id,
    });
    const roots = buildTaskTree(
      [a, b],
      relMap([
        [a.id, [bParentOfA, selfEdge]],
        [b.id, [aParentOfB]],
      ]),
    );
    // One of the two still nests; the cycle-closing edge is refused.
    const total = roots.reduce((n, root) => n + 1 + root.children.length, 0);
    expect(total).toBe(2);
    expect(roots.length).toBeGreaterThanOrEqual(1);
  });

  it("preserves page order for roots", () => {
    const first = task({ title: "First" });
    const second = task({ title: "Second" });
    const roots = buildTaskTree(
      [first, second],
      relMap([
        [first.id, []],
        [second.id, []],
      ]),
    );
    expect(roots.map((r) => r.task.title)).toEqual(["First", "Second"]);
  });
});

describe("describeDependencies", () => {
  const statusMap = (entries: [string, TaskStatus][]) => new Map(entries);

  function nodeWith(dependsOnIds: string[]) {
    const t = task({});
    return {
      task: t,
      children: [],
      subtaskCount: 0,
      parentId: null,
      parentInView: false,
      dependsOnIds,
    };
  }

  it("returns null without dependencies", () => {
    expect(describeDependencies(nodeWith([]), statusMap([]))).toBeNull();
  });

  it("counts unmet dependencies when statuses are known", () => {
    const hint = describeDependencies(
      nodeWith(["d1", "d2", "d3"]),
      statusMap([
        ["d1", "done"],
        ["d2", "ready"],
        ["d3", "in_progress"],
      ]),
    );
    expect(hint).toEqual({ kind: "waits_on", count: 2 });
  });

  it("returns null when every dependency is done", () => {
    const hint = describeDependencies(nodeWith(["d1"]), statusMap([["d1", "done"]]));
    expect(hint).toBeNull();
  });

  it("falls back to a plain count when statuses are unknown", () => {
    const hint = describeDependencies(nodeWith(["d1", "d2"]), statusMap([["d1", "done"]]));
    expect(hint).toEqual({ kind: "dependencies", count: 2 });
  });
});
