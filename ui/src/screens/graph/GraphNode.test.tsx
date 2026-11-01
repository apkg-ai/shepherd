import { render, screen } from "@testing-library/react";
import { Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { describe, expect, it, vi } from "vitest";
import { makeGraphNode } from "../../test/fixtures";
import type { GraphNodeData, GraphNodeType } from "./GraphNode";
import { GraphNode } from "./GraphNode";

vi.mock("@xyflow/react", async () => {
  const actual = await vi.importActual<typeof import("@xyflow/react")>("@xyflow/react");
  return {
    ...actual,
    Handle: ({ type }: { type: string }) => <div data-testid={`handle-${type}`} />,
  };
});

function nodeProps(
  overrides?: Partial<GraphNodeData>,
): NodeProps<GraphNodeType> {
  const node = makeGraphNode(overrides);
  return {
    id: node.id,
    data: {
      ...node,
      selected: overrides?.selected ?? false,
      dimmed: overrides?.dimmed ?? false,
    },
    type: "graphNode",
    dragging: false,
    zIndex: 0,
    selectable: true,
    deletable: false,
    selected: false,
    draggable: true,
    isConnectable: true,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    width: 200,
    height: 100,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
  };
}

describe("GraphNode", () => {
  it("renders title", () => {
    render(<GraphNode {...nodeProps({ title: "Deploy service" })} />);
    expect(screen.getByText("Deploy service")).toBeInTheDocument();
  });

  it("renders status badge", () => {
    render(<GraphNode {...nodeProps({ status: "active" })} />);
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  it("renders progress counts when total > 0", () => {
    render(
      <GraphNode
        {...nodeProps({ counts: { total: 5, done: 2, cancelled: 0, waived: 0 } })}
      />,
    );
    expect(screen.getByText("2 of 5")).toBeInTheDocument();
  });

  it("does not render progress counts when total is 0", () => {
    render(
      <GraphNode
        {...nodeProps({ counts: { total: 0, done: 0, cancelled: 0, waived: 0 } })}
      />,
    );
    expect(screen.queryByText(/of/)).not.toBeInTheDocument();
  });

  it("applies selected state", () => {
    render(<GraphNode {...nodeProps({ selected: true })} />);
    const node = screen.getByLabelText(/Test node/);
    expect(node).toHaveAttribute("data-selected", "true");
  });

  it("applies dimmed state", () => {
    render(<GraphNode {...nodeProps({ dimmed: true })} />);
    const node = screen.getByLabelText(/Test node/);
    expect(node).toHaveAttribute("data-dimmed", "true");
  });

  it("has correct aria-label", () => {
    render(<GraphNode {...nodeProps({ title: "Build API", status: "open" })} />);
    expect(screen.getByLabelText("Build API — open")).toBeInTheDocument();
  });
});
