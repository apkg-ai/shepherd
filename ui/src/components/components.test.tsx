import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { ShepherdError } from "../api/problem";
import { Button } from "./Button";
import { ConfirmDialog } from "./ConfirmDialog";
import { CopyButton } from "./CopyButton";
import { Dialog } from "./Dialog";
import { FormField } from "./FormField";
import { LoadMore } from "./LoadMore";
import { PageHeader } from "./PageHeader";
import { ThemeToggle } from "./ThemeToggle";
import { EmptyState, ErrorState, LoadingState } from "./states";
import { ToastProvider, useToast } from "./Toast";

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
        title="Delete item"
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

describe("CopyButton", () => {
  // fireEvent, not userEvent: userEvent.setup() installs its own clipboard
  // stub, which would silently override the mocks these tests assert on.
  it("copies the value and confirms briefly", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });

    render(<CopyButton value="secret-value" label="Copy the value" />);
    const button = screen.getByRole("button", { name: "Copy the value" });
    expect(button).toHaveTextContent("Copy");

    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent("Copied ✓"));
    expect(writeText).toHaveBeenCalledWith("secret-value");
  });

  it("stays quiet when the clipboard is unavailable", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });

    render(<CopyButton value="secret-value" label="Copy the value" />);
    const button = screen.getByRole("button", { name: "Copy the value" });
    fireEvent.click(button);
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(button).toHaveTextContent("Copy");
  });
});

describe("PageHeader", () => {
  it("renders title, description, actions, and children slots", () => {
    render(
      <PageHeader
        title="Shepherd"
        description="v1 scaffold"
        actions={<button type="button">Act</button>}
      >
        <p>slot content</p>
      </PageHeader>,
    );
    expect(screen.getByRole("heading", { name: "Shepherd" })).toBeInTheDocument();
    expect(screen.getByText("v1 scaffold")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Act" })).toBeInTheDocument();
    expect(screen.getByText("slot content")).toBeInTheDocument();
  });

  it("omits the description paragraph when absent", () => {
    render(<PageHeader title="Bare" />);
    expect(screen.getByRole("heading", { name: "Bare" })).toBeInTheDocument();
    expect(screen.queryByRole("paragraph")).not.toBeInTheDocument();
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

describe("ThemeToggle", () => {
  it("applies and persists the picked theme", async () => {
    const user = userEvent.setup();
    render(<ThemeToggle />);

    await user.click(screen.getByRole("button", { name: "Dark" }));
    expect(document.documentElement.dataset["theme"]).toBe("dark");
    expect(localStorage.getItem("shepherd-theme")).toBe("dark");
    expect(screen.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByRole("button", { name: "Light" }));
    expect(document.documentElement.dataset["theme"]).toBe("light");

    // System resolves to light in jsdom (no matchMedia → not dark).
    await user.click(screen.getByRole("button", { name: "System" }));
    expect(document.documentElement.dataset["theme"]).toBe("light");
    localStorage.clear();
    delete document.documentElement.dataset["theme"];
  });
});

describe("Dialog focus management", () => {
  function Host() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button type="button" onClick={() => setOpen(true)}>
          opener
        </button>
        <Dialog open={open} title="Trapped" onClose={() => setOpen(false)}>
          <input type="text" aria-label="First field" />
          <button type="button">Last action</button>
        </Dialog>
      </>
    );
  }

  it("traps Tab inside the panel and wraps around", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(screen.getByRole("button", { name: "opener" }));
    const dialog = screen.getByRole("dialog", { name: "Trapped" });
    expect(dialog).toHaveFocus();

    // Panel DOM order: header Close button, then the children.
    await user.tab();
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    await user.tab();
    expect(screen.getByLabelText("First field")).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Last action" })).toHaveFocus();
    // Tab from the last focusable wraps back to the first — never escapes.
    await user.tab();
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
  });

  it("wraps backwards with Shift+Tab from the first element", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(screen.getByRole("button", { name: "opener" }));
    await user.tab(); // first focusable
    await user.tab({ shift: true }); // back past the start → wraps to last
    expect(screen.getByRole("button", { name: "Last action" })).toHaveFocus();
  });

  it("returns focus to the opener on close", async () => {
    const user = userEvent.setup();
    render(<Host />);
    const opener = screen.getByRole("button", { name: "opener" });
    await user.click(opener);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
  });

  it("closes on outside mousedown but not inside", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(screen.getByRole("button", { name: "opener" }));
    await user.click(screen.getByLabelText("First field"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.click(document.body);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("Toast pause (WCAG 2.2.1)", () => {
  function Probe() {
    const { toast } = useToast();
    return (
      <button type="button" onClick={() => toast("Hold me", "info")}>
        fire
      </button>
    );
  }

  it("pauses auto-dismiss on hover and restarts on leave", () => {
    vi.useFakeTimers();
    try {
      render(
        <ToastProvider>
          <Probe />
        </ToastProvider>,
      );
      fireEvent.click(screen.getByRole("button", { name: "fire" }));
      const toast = screen.getByRole("status");

      fireEvent.mouseEnter(toast);
      act(() => vi.advanceTimersByTime(20000));
      expect(screen.getByRole("status")).toBeInTheDocument();

      fireEvent.mouseLeave(toast);
      act(() => vi.advanceTimersByTime(5000));
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("pauses while the dismiss button is focused", () => {
    vi.useFakeTimers();
    try {
      render(
        <ToastProvider>
          <Probe />
        </ToastProvider>,
      );
      fireEvent.click(screen.getByRole("button", { name: "fire" }));
      fireEvent.focus(screen.getByRole("button", { name: "Dismiss" }));
      act(() => vi.advanceTimersByTime(20000));
      expect(screen.getByRole("status")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("FormField hint exposure", () => {
  it("links the hint via aria-describedby when no error", () => {
    render(
      <FormField label="Name" hint="Keep it short">
        {(props) => <input {...props} type="text" />}
      </FormField>,
    );
    const input = screen.getByLabelText("Name");
    const hintId = input.getAttribute("aria-describedby");
    expect(hintId).toBeTruthy();
    expect(document.getElementById(hintId!)).toHaveTextContent("Keep it short");
  });

  it("prefers the error id when an error is shown", () => {
    render(
      <FormField label="Name" hint="Keep it short" error="required">
        {(props) => <input {...props} type="text" />}
      </FormField>,
    );
    const input = screen.getByLabelText("Name");
    const describedBy = input.getAttribute("aria-describedby");
    expect(document.getElementById(describedBy!)).toHaveTextContent("required");
  });
});
