import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";
import { I18nextProvider } from "react-i18next";
import i18next from "i18next";
import { PhaseBadge } from "./PhaseBadge";
import { ProgressCounts } from "./ProgressCounts";
import { EligibilityPanel } from "./EligibilityPanel";
import { ConflictView } from "./ConflictView";
import { ReasonDialog } from "./ReasonDialog";
import type { Eligibility } from "../api/generated/model/eligibility";

// Reuse the app's configured i18next instance
import "../i18n";

function wrap(ui: React.ReactElement) {
  return render(
    <I18nextProvider i18n={i18next}>
      <MemoryRouter>{ui}</MemoryRouter>
    </I18nextProvider>,
  );
}

describe("PhaseBadge", () => {
  it("renders known phase with translated label", () => {
    wrap(<PhaseBadge phase="planning" />);
    expect(screen.getByText("Planning")).toBeInTheDocument();
  });

  it("renders plan_review phase", () => {
    wrap(<PhaseBadge phase="plan_review" />);
    expect(screen.getByText("Plan review")).toBeInTheDocument();
  });

  it("renders execution phase", () => {
    wrap(<PhaseBadge phase="execution" />);
    expect(screen.getByText("Execution")).toBeInTheDocument();
  });

  it("renders work_review phase", () => {
    wrap(<PhaseBadge phase="work_review" />);
    expect(screen.getByText("Work review")).toBeInTheDocument();
  });

  it("renders complete phase", () => {
    wrap(<PhaseBadge phase="complete" />);
    expect(screen.getByText("Complete")).toBeInTheDocument();
  });

  it("falls back to raw value for unknown phase", () => {
    wrap(<PhaseBadge phase="unknown_phase" />);
    expect(screen.getByText("unknown_phase")).toBeInTheDocument();
  });

  it("sets data-phase attribute for CSS targeting", () => {
    wrap(<PhaseBadge phase="execution" />);
    expect(screen.getByText("Execution")).toHaveAttribute("data-phase", "execution");
  });
});

describe("ProgressCounts", () => {
  it("renders done of total", () => {
    wrap(<ProgressCounts counts={{ total: 10, done: 3, cancelled: 0, waived: 0 }} />);
    expect(screen.getByText("3 of 10")).toBeInTheDocument();
  });

  it("distinguishes cancelled and waived", () => {
    wrap(<ProgressCounts counts={{ total: 10, done: 5, cancelled: 2, waived: 1 }} />);
    const text = screen.getByText(/5 of 10/).textContent!;
    expect(text).toContain("2 cancelled");
    expect(text).toContain("1 waived");
  });

  it("omits cancelled when zero", () => {
    wrap(<ProgressCounts counts={{ total: 5, done: 2, cancelled: 0, waived: 1 }} />);
    const text = screen.getByText(/2 of 5/).textContent!;
    expect(text).not.toContain("cancelled");
    expect(text).toContain("1 waived");
  });

  it("omits waived when zero", () => {
    wrap(<ProgressCounts counts={{ total: 5, done: 2, cancelled: 1, waived: 0 }} />);
    const text = screen.getByText(/2 of 5/).textContent!;
    expect(text).toContain("1 cancelled");
    expect(text).not.toContain("waived");
  });

  it("handles all-zero counts", () => {
    wrap(<ProgressCounts counts={{ total: 0, done: 0, cancelled: 0, waived: 0 }} />);
    expect(screen.getByText("0 of 0")).toBeInTheDocument();
  });
});

describe("EligibilityPanel", () => {
  const baseEligibility: Eligibility = {
    can_plan: false,
    can_execute: false,
    can_review: false,
    reasons: [],
    allowed_actions: [],
  };

  it("renders capability flags", () => {
    wrap(
      <EligibilityPanel
        eligibility={{ ...baseEligibility, can_plan: true, can_execute: true }}
        projectId="p1"
      />,
    );
    expect(screen.getByText("Can plan")).toBeInTheDocument();
    expect(screen.getByText("Can execute")).toBeInTheDocument();
    expect(screen.getByText("Can review")).toBeInTheDocument();
  });

  it("shows no blockers message when reasons empty", () => {
    wrap(<EligibilityPanel eligibility={baseEligibility} projectId="p1" />);
    expect(screen.getByText("No blockers")).toBeInTheDocument();
  });

  it("renders multiple gate reasons with links", () => {
    const eligibility: Eligibility = {
      ...baseEligibility,
      reasons: [
        { code: "task_prerequisite", resource_id: "t1", message: "Blocked by task A" },
        { code: "epic_block", resource_id: "e1", message: "Epic is blocked" },
        { code: "wrong_phase", resource_id: "t2", message: "Wrong phase" },
      ],
    };
    wrap(<EligibilityPanel eligibility={eligibility} projectId="p1" />);

    expect(screen.getByText("Blocked by task A")).toBeInTheDocument();
    expect(screen.getByText("Epic is blocked")).toBeInTheDocument();
    expect(screen.getByText("Wrong phase")).toBeInTheDocument();

    // task_prerequisite and epic_block should be links
    expect(screen.getByRole("link", { name: "Blocked by task A" })).toHaveAttribute(
      "href",
      "/projects/p1/tasks/t1",
    );
    expect(screen.getByRole("link", { name: "Epic is blocked" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1",
    );

    // wrong_phase has no resource link — just text
    expect(screen.queryByRole("link", { name: "Wrong phase" })).not.toBeInTheDocument();
  });

  it("has accessible section label", () => {
    wrap(<EligibilityPanel eligibility={baseEligibility} projectId="p1" />);
    expect(screen.getByRole("region", { name: "Eligibility" })).toBeInTheDocument();
  });
});

describe("ConflictView", () => {
  it("renders local and remote field values", () => {
    const onRetry = vi.fn();
    const onDiscard = vi.fn();
    wrap(
      <ConflictView
        fields={[
          { label: "Title", local: "My title", remote: "Their title" },
          { label: "Description", local: "My desc", remote: "Their desc" },
        ]}
        onRetry={onRetry}
        onDiscard={onDiscard}
      />,
    );

    expect(screen.getByText("Your version")).toBeInTheDocument();
    expect(screen.getByText("Current version")).toBeInTheDocument();
    expect(screen.getByText("My title")).toBeInTheDocument();
    expect(screen.getByText("Their title")).toBeInTheDocument();
  });

  it("calls onRetry when Save anyway is clicked", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    wrap(
      <ConflictView
        fields={[{ label: "Title", local: "a", remote: "b" }]}
        onRetry={onRetry}
        onDiscard={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Save anyway" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("calls onDiscard when Discard is clicked", async () => {
    const user = userEvent.setup();
    const onDiscard = vi.fn();
    wrap(
      <ConflictView
        fields={[{ label: "Title", local: "a", remote: "b" }]}
        onRetry={vi.fn()}
        onDiscard={onDiscard}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Discard my changes" }));
    expect(onDiscard).toHaveBeenCalledOnce();
  });

  it("shows empty label for missing values", () => {
    wrap(
      <ConflictView
        fields={[{ label: "Description", local: "", remote: "" }]}
        onRetry={vi.fn()}
        onDiscard={vi.fn()}
      />,
    );

    expect(screen.getAllByText("—")).toHaveLength(2);
  });
});

describe("ReasonDialog", () => {
  it("renders reason textarea and submit button", () => {
    wrap(<ReasonDialog open title="Block task" onSubmit={vi.fn()} onCancel={vi.fn()} />);

    expect(screen.getByLabelText("Reason")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Submit" })).toBeInTheDocument();
  });

  it("submit is disabled when reason is empty", () => {
    wrap(<ReasonDialog open title="Block task" onSubmit={vi.fn()} onCancel={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Submit" })).toBeDisabled();
  });

  it("calls onSubmit with trimmed reason", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    wrap(<ReasonDialog open title="Block task" onSubmit={onSubmit} onCancel={vi.fn()} />);

    await user.type(screen.getByLabelText("Reason"), "  Dependency missing  ");
    await user.click(screen.getByRole("button", { name: "Submit" }));
    expect(onSubmit).toHaveBeenCalledWith("Dependency missing");
  });

  it("does not render when closed", () => {
    wrap(<ReasonDialog open={false} title="Block task" onSubmit={vi.fn()} onCancel={vi.fn()} />);

    expect(screen.queryByLabelText("Reason")).not.toBeInTheDocument();
  });
});
