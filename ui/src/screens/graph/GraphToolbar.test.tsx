import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { GraphToolbar } from "./GraphToolbar";

function renderToolbar(overrides: Partial<Parameters<typeof GraphToolbar>[0]> = {}) {
  const props = {
    hasSelection: false,
    floating: true,
    onFit: vi.fn(),
    onZoomIn: vi.fn(),
    onZoomOut: vi.fn(),
    onResetLayout: vi.fn(),
    onAddDependency: vi.fn(),
    ...overrides,
  };
  render(<GraphToolbar {...props} />);
  return props;
}

describe("GraphToolbar", () => {
  it("renders all icon buttons", () => {
    renderToolbar();

    expect(screen.getByRole("group", { name: "Graph controls" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Fit to view" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom out" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset layout" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add dependency" })).toBeInTheDocument();
  });

  it("add dependency disabled when no selection", () => {
    renderToolbar({ hasSelection: false });

    expect(screen.getByRole("button", { name: "Add dependency" })).toBeDisabled();
  });

  it("add dependency enabled when selection exists", () => {
    renderToolbar({ hasSelection: true });

    expect(screen.getByRole("button", { name: "Add dependency" })).toBeEnabled();
  });

  it("shows the reset button when a reset handler is provided", () => {
    renderToolbar();

    expect(screen.getByRole("button", { name: "Reset layout" })).toBeInTheDocument();
  });

  it("hides the reset button when no reset handler is provided", () => {
    render(
      <GraphToolbar
        hasSelection={false}
        floating
        onFit={vi.fn()}
        onZoomIn={vi.fn()}
        onZoomOut={vi.fn()}
        onAddDependency={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "Reset layout" })).not.toBeInTheDocument();
  });

  it("labels view toggles by destination in table mode", () => {
    renderToolbar({
      viewMode: "table",
      onToggleView: vi.fn(),
      onToggleTable: vi.fn(),
    });

    expect(screen.getByRole("button", { name: "List view" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Graph view" })).toBeInTheDocument();
  });

  it("labels view toggles by destination in list mode", () => {
    renderToolbar({
      viewMode: "list",
      onToggleView: vi.fn(),
      onToggleTable: vi.fn(),
    });

    expect(screen.getByRole("button", { name: "Graph view" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Table view" })).toBeInTheDocument();
  });

  it("fires callbacks", async () => {
    const user = userEvent.setup();
    const props = renderToolbar({ hasSelection: true });

    await user.click(screen.getByRole("button", { name: "Fit to view" }));
    expect(props.onFit).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(props.onZoomIn).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(props.onZoomOut).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Reset layout" }));
    expect(props.onResetLayout).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Add dependency" }));
    expect(props.onAddDependency).toHaveBeenCalledOnce();
  });
});
