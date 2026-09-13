import { describe, expect, it } from "vitest";
import { relation } from "../test/fixtures";
import { computeNeighborIds } from "./graphNeighbors";

describe("computeNeighborIds", () => {
  describe("depends_on lens", () => {
    it("returns only the selected node when there are no edges", () => {
      const result = computeNeighborIds("a", [], "depends_on");
      expect(result).toEqual(new Set(["a"]));
    });

    it("follows the full transitive upstream chain (prerequisites)", () => {
      // C depends on B, B depends on A → selecting C yields {A, B, C}
      const rels = [
        relation({ type: "depends_on", source_task_id: "c", target_task_id: "b" }),
        relation({ type: "depends_on", source_task_id: "b", target_task_id: "a" }),
      ];
      const result = computeNeighborIds("c", rels, "depends_on");
      expect(result).toEqual(new Set(["a", "b", "c"]));
    });

    it("follows the full transitive downstream chain (dependents)", () => {
      // C depends on B, B depends on A → selecting A yields {A, B, C}
      const rels = [
        relation({ type: "depends_on", source_task_id: "c", target_task_id: "b" }),
        relation({ type: "depends_on", source_task_id: "b", target_task_id: "a" }),
      ];
      const result = computeNeighborIds("a", rels, "depends_on");
      expect(result).toEqual(new Set(["a", "b", "c"]));
    });

    it("combines upstream and downstream from a middle node", () => {
      // D→C→B→A — selecting B yields the whole chain
      const rels = [
        relation({ type: "depends_on", source_task_id: "d", target_task_id: "c" }),
        relation({ type: "depends_on", source_task_id: "c", target_task_id: "b" }),
        relation({ type: "depends_on", source_task_id: "b", target_task_id: "a" }),
      ];
      const result = computeNeighborIds("b", rels, "depends_on");
      expect(result).toEqual(new Set(["a", "b", "c", "d"]));
    });

    it("ignores relations of the wrong type", () => {
      const rels = [
        relation({ type: "decomposition", source_task_id: "a", target_task_id: "b" }),
      ];
      const result = computeNeighborIds("a", rels, "depends_on");
      expect(result).toEqual(new Set(["a"]));
    });

    it("excludes disconnected nodes", () => {
      // B depends on A, D is separate
      const rels = [
        relation({ type: "depends_on", source_task_id: "b", target_task_id: "a" }),
      ];
      const result = computeNeighborIds("a", rels, "depends_on");
      expect(result).not.toContain("d");
    });

    it("handles self-edges gracefully", () => {
      const rels = [
        relation({ type: "depends_on", source_task_id: "a", target_task_id: "a" }),
      ];
      const result = computeNeighborIds("a", rels, "depends_on");
      expect(result).toEqual(new Set(["a"]));
    });

    it("highlights an epic blocked by two prerequisites", () => {
      // Epic C depends_on Epic A AND Epic B.
      const rels = [
        relation({ type: "depends_on", source_task_id: "epicC", target_task_id: "epicA" }),
        relation({ type: "depends_on", source_task_id: "epicC", target_task_id: "epicB" }),
      ];
      // Select C → both blockers are in the cone.
      expect(computeNeighborIds("epicC", rels, "depends_on")).toEqual(
        new Set(["epicA", "epicB", "epicC"]),
      );
      // Select A → C is downstream, but B is not related to A.
      expect(computeNeighborIds("epicA", rels, "depends_on")).toEqual(
        new Set(["epicA", "epicC"]),
      );
    });

    it("follows cross-epic task deps without pulling in decomposition siblings", () => {
      // Task in Epic 1 depends on a task in Epic 2; both also have decomposition
      // siblings, but those are invisible to the depends_on lens.
      const rels = [
        relation({ type: "depends_on", source_task_id: "t1.1", target_task_id: "t2.1" }),
        relation({ type: "decomposition", source_task_id: "epic1", target_task_id: "t1.1" }),
        relation({ type: "decomposition", source_task_id: "epic1", target_task_id: "t1.2" }),
        relation({ type: "decomposition", source_task_id: "epic2", target_task_id: "t2.1" }),
        relation({ type: "decomposition", source_task_id: "epic2", target_task_id: "t2.2" }),
      ];
      const result = computeNeighborIds("t1.1", rels, "depends_on");
      // Only the direct dependency pair — no siblings, no epics.
      expect(result).toEqual(new Set(["t1.1", "t2.1"]));
    });
  });

  describe("decomposition lens", () => {
    it("walks up the parent chain and down the subtree", () => {
      // Root → Epic → Task
      const rels = [
        relation({ type: "decomposition", source_task_id: "root", target_task_id: "epic" }),
        relation({ type: "decomposition", source_task_id: "epic", target_task_id: "task" }),
      ];
      // Selecting "epic" should include root (parent) and task (child).
      const result = computeNeighborIds("epic", rels, "decomposition");
      expect(result).toEqual(new Set(["root", "epic", "task"]));
    });

    it("includes ancestors and descendants but not sibling branches", () => {
      // Root → Epic1 → T1, Root → Epic2 → T2
      const rels = [
        relation({ type: "decomposition", source_task_id: "root", target_task_id: "epic1" }),
        relation({ type: "decomposition", source_task_id: "root", target_task_id: "epic2" }),
        relation({ type: "decomposition", source_task_id: "epic1", target_task_id: "t1" }),
        relation({ type: "decomposition", source_task_id: "epic2", target_task_id: "t2" }),
      ];
      // Selecting "t1": ancestors are epic1 → root. No descendants.
      // Epic2 and t2 are in a different branch — not in the cone.
      const result = computeNeighborIds("t1", rels, "decomposition");
      expect(result).toEqual(new Set(["root", "epic1", "t1"]));
    });

    it("selecting a parent includes all descendants", () => {
      const rels = [
        relation({ type: "decomposition", source_task_id: "root", target_task_id: "epic1" }),
        relation({ type: "decomposition", source_task_id: "root", target_task_id: "epic2" }),
        relation({ type: "decomposition", source_task_id: "epic1", target_task_id: "t1" }),
      ];
      // Selecting "root": descendants are epic1, epic2, t1.
      const result = computeNeighborIds("root", rels, "decomposition");
      expect(result).toEqual(new Set(["root", "epic1", "epic2", "t1"]));
    });
  });
});
