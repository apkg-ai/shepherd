import { render, screen } from "@testing-library/react";
import { Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { describe, expect, it, vi } from "vitest";
import { DagEndpointNode, type DagEndpointNodeType } from "./DagEndpointNode";

vi.mock("@xyflow/react", async () => {
  const actual = await vi.importActual<typeof import("@xyflow/react")>("@xyflow/react");
  return {
    ...actual,
    Handle: ({ type }: { type: string }) => <div data-testid={`handle-${type}`} />,
  };
});

function endpointProps(label: "start" | "end"): NodeProps<DagEndpointNodeType> {
  return {
    id: `__dag_${label}__`,
    data: { label },
    type: "dagEndpoint",
    dragging: false,
    zIndex: 0,
    selectable: true,
    deletable: false,
    selected: false,
    draggable: true,
    isConnectable: true,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    width: 120,
    height: 50,
    sourcePosition: Position.Bottom,
    targetPosition: Position.Top,
  };
}

describe("DagEndpointNode", () => {
  it("renders the start label with only a source handle", () => {
    render(<DagEndpointNode {...endpointProps("start")} />);
    expect(screen.getByText("start")).toBeInTheDocument();
    expect(screen.getByTestId("handle-source")).toBeInTheDocument();
    expect(screen.queryByTestId("handle-target")).not.toBeInTheDocument();
  });

  it("renders the end label with only a target handle", () => {
    render(<DagEndpointNode {...endpointProps("end")} />);
    expect(screen.getByText("end")).toBeInTheDocument();
    expect(screen.getByTestId("handle-target")).toBeInTheDocument();
    expect(screen.queryByTestId("handle-source")).not.toBeInTheDocument();
  });
});
