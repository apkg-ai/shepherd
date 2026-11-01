import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { GraphToolbar } from "./GraphToolbar";

function renderToolbar(overrides: Partial<Parameters<typeof GraphToolbar>[0]> = {}) {
  const props = {
    viewMode: "graph" as const,
    hasSelection: false,
    onFit: vi.fn(),
    onZoomIn: vi.fn(),
    onZoomOut: vi.fn(),
    onResetLayout: vi.fn(),
    onToggleView: vi.fn(),
    onAddDependency: vi.fn(),
    ...overrides,
  };
  render(<GraphToolbar {...props} />);
  return props;
}

describe("GraphToolbar", () => {
  it("renders all graph mode buttons", () => {
    renderToolbar();

    expect(screen.getByRole("toolbar", { name: "Graph controls" })).toBeInTheDocument();
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

  it("toggle shows List view in graph mode", () => {
    renderToolbar({ viewMode: "graph" });

    expect(screen.getByRole("button", { name: "List view" })).toBeInTheDocument();
  });

  it("toggle shows Graph view in list mode", () => {
    renderToolbar({ viewMode: "list" });

    expect(screen.getByRole("button", { name: "Graph view" })).toBeInTheDocument();
  });

  it("hides graph controls in list mode", () => {
    renderToolbar({ viewMode: "list" });

    expect(screen.queryByRole("button", { name: "Fit to view" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Zoom in" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Zoom out" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reset layout" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add dependency" })).not.toBeInTheDocument();
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

    await user.click(screen.getByRole("button", { name: "List view" }));
    expect(props.onToggleView).toHaveBeenCalledOnce();
  });
});
