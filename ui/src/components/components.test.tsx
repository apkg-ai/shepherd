import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import type { TaskStatus } from "../api/generated/model";
import { ShepherdError } from "../api/problem";
import { AttemptBadge } from "./AttemptBadge";
import { Button } from "./Button";
import { ConfirmDialog } from "./ConfirmDialog";
import { Dialog } from "./Dialog";
import { FormField } from "./FormField";
import { IdentityChip } from "./IdentityChip";
import { LoadMore } from "./LoadMore";
import { ReasonDialog } from "./ReasonDialog";
import { StatusBadge } from "./StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "./states";
import { ToastProvider, useToast } from "./Toast";

describe("StatusBadge", () => {
  const cases: [TaskStatus, string][] = [
    ["proposed", "Proposed"],
    ["approved", "Approved"],
    ["ready", "Ready"],
    ["in_progress", "In progress"],
    ["in_review", "In review"],
    ["done", "Done"],
    ["blocked", "Blocked"],
    ["cancelled", "Cancelled"],
  ];
  it.each(cases)("renders %s with its data-status", (status, label) => {
    render(<StatusBadge status={status} />);
    const badge = screen.getByText(label);
    expect(badge).toHaveAttribute("data-status", status);
  });
});

describe("AttemptBadge", () => {
  it("renders nothing before the first attempt", () => {
    const { container } = render(<AttemptBadge attempts={0} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders singular and plural attempt counts", () => {
    render(<AttemptBadge attempts={1} />);
    expect(screen.getByText("1 attempt")).toBeInTheDocument();
    render(<AttemptBadge attempts={3} />);
    expect(screen.getByText("3 attempts")).toBeInTheDocument();
  });

  it("surfaces failures", () => {
    render(<AttemptBadge attempts={4} failures={2} />);
    expect(screen.getByText("4 attempts, 2 failed")).toBeInTheDocument();
  });
});

describe("IdentityChip", () => {
  const base = { harness: "claude-code", agent_model: "opus-5", session_id: "s1" };

  it("prefers the label", () => {
    render(<IdentityChip identity={{ ...base, label: "Reviewer" }} />);
    expect(screen.getByText("Reviewer")).toBeInTheDocument();
  });

  it("falls back to model · harness", () => {
    render(<IdentityChip identity={base} />);
    expect(screen.getByText("opus-5 · claude-code")).toBeInTheDocument();
  });
});

describe("Button", () => {
  it("disables and marks busy", () => {
    render(<Button busy>Save</Button>);
    const button = screen.getByRole("button");
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
  });
});

describe("Dialog", () => {
  it("renders nothing when closed", () => {
    render(
      <Dialog open={false} title="Hidden" onClose={() => {}}>
        <p>content</p>
      </Dialog>,
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("closes on Escape and on the close button", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <Dialog open title="Visible" onClose={onClose}>
        <p>content</p>
      </Dialog>,
    );
    expect(screen.getByRole("dialog", { name: "Visible" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("ConfirmDialog", () => {
  it("wires confirm and cancel", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        open
        title="Delete task"
        confirmLabel="Delete"
        danger
        onConfirm={onConfirm}
        onCancel={onCancel}
      >
        Are you sure?
      </ConfirmDialog>,
    );
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe("FormField", () => {
  it("wires label and error to the control", () => {
    render(
      <FormField label="Title" error="required">
        {(props) => <input {...props} type="text" />}
      </FormField>,
    );
    const input = screen.getByLabelText("Title");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("required")).toBeInTheDocument();
  });

  it("shows the hint only while there is no error", () => {
    const { rerender } = render(
      <FormField label="Title" hint="Keep it short">
        {(props) => <input {...props} type="text" />}
      </FormField>,
    );
    expect(screen.getByText("Keep it short")).toBeInTheDocument();
    rerender(
      <FormField label="Title" hint="Keep it short" error="required">
        {(props) => <input {...props} type="text" />}
      </FormField>,
    );
    expect(screen.queryByText("Keep it short")).not.toBeInTheDocument();
  });
});

describe("LoadMore", () => {
  it("renders nothing without a next page", () => {
    const { container } = render(
      <LoadMore hasNextPage={false} isFetchingNextPage={false} onLoadMore={() => {}} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("loads more on click", async () => {
    const user = userEvent.setup();
    const onLoadMore = vi.fn();
    render(<LoadMore hasNextPage isFetchingNextPage={false} onLoadMore={onLoadMore} />);
    await user.click(screen.getByRole("button", { name: "Load more" }));
    expect(onLoadMore).toHaveBeenCalledOnce();
  });
});

describe("states", () => {
  it("LoadingState exposes a status role", () => {
    render(<LoadingState />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading…");
  });

  it("EmptyState renders title and children", () => {
    render(
      <EmptyState title="Nothing here">
        <p>hint</p>
      </EmptyState>,
    );
    expect(screen.getByText("Nothing here")).toBeInTheDocument();
    expect(screen.getByText("hint")).toBeInTheDocument();
  });

  it("ErrorState renders ShepherdError title/detail and retries", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    render(
      <ErrorState
        error={
          new ShepherdError({
            type: "urn:shepherd:error:internal-error",
            title: "Server exploded",
            status: 500,
            detail: "Try again later",
          })
        }
        onRetry={onRetry}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Server exploded");
    expect(screen.getByText("Try again later")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("ErrorState falls back for unknown errors", () => {
    render(<ErrorState error={new Error("boom")} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Something went wrong");
  });
});

describe("Toast", () => {
  function Probe() {
    const { toast } = useToast();
    return (
      <button type="button" onClick={() => toast("Saved!", "success")}>
        fire
      </button>
    );
  }

  it("shows, dismisses manually, and auto-dismisses", () => {
    vi.useFakeTimers();
    try {
      render(
        <ToastProvider>
          <Probe />
        </ToastProvider>,
      );
      fireEvent.click(screen.getByRole("button", { name: "fire" }));
      expect(screen.getByRole("status")).toHaveTextContent("Saved!");

      fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByRole("status")).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: "fire" }));
      act(() => vi.advanceTimersByTime(5000));
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("throws when used outside the provider", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => render(<Probe />)).toThrow(/within ToastProvider/);
    spy.mockRestore();
  });
});

describe("ReasonDialog", () => {
  it("resets reason and error state on every open", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    function Host() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            open
          </button>
          <ReasonDialog
            open={open}
            title="Reject work"
            label="Reason"
            confirmLabel="Reject"
            validate={(reason) => (reason ? null : "required")}
            onSubmit={onSubmit}
            onCancel={() => setOpen(false)}
          />
        </>
      );
    }
    render(<Host />);

    // First open: trigger a validation error, then type a reason, then cancel.
    await user.click(screen.getByRole("button", { name: "open" }));
    await user.click(screen.getByRole("button", { name: "Reject" }));
    expect(screen.getByText("required")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Reason"), "task A's reason");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    // Reopen (e.g. for a different task): both reason and error are gone.
    await user.click(screen.getByRole("button", { name: "open" }));
    expect(screen.getByLabelText("Reason")).toHaveValue("");
    expect(screen.queryByText("required")).not.toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});

describe("AttemptBadge atLeast", () => {
  it("marks partially counted failures with ≥", () => {
    render(<AttemptBadge attempts={30} failures={4} atLeast />);
    expect(screen.getByText("30 attempts, ≥4 failed")).toBeInTheDocument();
  });

  it("claims exact counts when fully loaded", () => {
    render(<AttemptBadge attempts={5} failures={2} atLeast={false} />);
    expect(screen.getByText("5 attempts, 2 failed")).toBeInTheDocument();
  });
});
