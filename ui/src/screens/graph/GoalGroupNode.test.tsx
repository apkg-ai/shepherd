import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { describe, expect, it, vi } from "vitest";
import { GoalGroupNode, type GoalGroupNodeType } from "./GoalGroupNode";

vi.mock("@xyflow/react", async () => {
  const actual = await vi.importActual<typeof import("@xyflow/react")>("@xyflow/react");
  return {
    ...actual,
    Handle: ({ type }: { type: string }) => <div data-testid={`handle-${type}`} />,
  };
});

function goalGroupProps(
  overrides?: Partial<GoalGroupNodeType["data"]>,
): NodeProps<GoalGroupNodeType> {
  const data = {
    title: "Launch the game",
    completed: false,
    epicCounts: { total: 4, done: 1, cancelled: 0, waived: 0 },
    expanded: false,
    onToggleExpand: vi.fn(),
    goalId: "g1",
    ...overrides,
  };
  return {
    id: "g1",
    data,
    type: "goalGroup",
    dragging: false,
    zIndex: 0,
    selectable: true,
    deletable: false,
    selected: false,
    draggable: true,
    isConnectable: true,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    width: 280,
    height: 60,
    sourcePosition: Position.Bottom,
    targetPosition: Position.Top,
  };
}

describe("GoalGroupNode", () => {
  it("renders the goal title with epic counts", () => {
    render(<GoalGroupNode {...goalGroupProps()} />);
    expect(screen.getByText("Launch the game")).toBeInTheDocument();
    expect(screen.getByText("1 of 4")).toBeInTheDocument();
  });

  it("shows the done status for a completed goal", () => {
    render(<GoalGroupNode {...goalGroupProps({ completed: true })} />);
    expect(screen.getByText("Done")).toBeInTheDocument();
  });

  it("expands via the toggle button", async () => {
    const user = userEvent.setup();
    const onToggleExpand = vi.fn();
    render(<GoalGroupNode {...goalGroupProps({ onToggleExpand })} />);

    await user.click(screen.getByRole("button", { name: "Show epics" }));
    expect(onToggleExpand).toHaveBeenCalledWith("g1");
  });

  it("labels the toggle as collapse when expanded", () => {
    render(<GoalGroupNode {...goalGroupProps({ expanded: true })} />);
    expect(screen.getByRole("button", { name: "Hide epics" })).toBeInTheDocument();
  });
});
