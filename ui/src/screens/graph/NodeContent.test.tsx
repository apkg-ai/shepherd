import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { NodeContent } from "./NodeContent";

const baseProps = {
  title: "Deploy service",
  status: "active",
  counts: { total: 0, done: 0, cancelled: 0, waived: 0 },
  expandable: false,
  expanded: false,
  selected: false,
  dimmed: false,
};

describe("NodeContent", () => {
  it("renders the title and status badge", () => {
    render(<NodeContent {...baseProps} />);
    expect(screen.getByText("Deploy service")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  it("renders progress counts when total > 0", () => {
    render(<NodeContent {...baseProps} counts={{ total: 5, done: 2, cancelled: 0, waived: 0 }} />);
    expect(screen.getByText("2 of 5")).toBeInTheDocument();
  });

  it("omits progress counts when total is 0", () => {
    render(<NodeContent {...baseProps} />);
    expect(screen.queryByText(/of/)).not.toBeInTheDocument();
  });

  it("hides the expand button when not expandable", () => {
    render(<NodeContent {...baseProps} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("hides the expand button when there is no handler", () => {
    render(<NodeContent {...baseProps} expandable />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("labels the expand button from the expanded state", async () => {
    const onExpandClick = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <NodeContent {...baseProps} expandable onExpandClick={onExpandClick} />,
    );

    const button = screen.getByRole("button", { name: "Show tasks" });
    await user.click(button);
    expect(onExpandClick).toHaveBeenCalledOnce();

    rerender(<NodeContent {...baseProps} expandable expanded onExpandClick={onExpandClick} />);
    expect(screen.getByRole("button", { name: "Hide tasks" })).toBeInTheDocument();
  });

  it("prefers the explicit expand aria-label", () => {
    render(
      <NodeContent
        {...baseProps}
        expandable
        onExpandClick={vi.fn()}
        expandAriaLabel="Show epics"
      />,
    );
    expect(screen.getByRole("button", { name: "Show epics" })).toBeInTheDocument();
  });
});
