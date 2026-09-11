import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, type RequestHandler } from "msw";
import { describe, expect, it } from "vitest";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import {
  getApproveTaskMockHandler,
  getCancelTaskMockHandler,
  getListTasksMockHandler,
  getRejectTaskMockHandler,
} from "../../api/generated/tasks/tasks.msw";
import { identity, page, project, task } from "../../test/fixtures";
import { problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("Review queue", () => {
  const proj = project({ name: "Alpha" });

  const inReview = task({
    title: "Needs review",
    status: "in_review",
    assignee: identity({ label: "Agent A" }),
    attempt_count: 1,
  });
  const proposed = task({ title: "Agent suggestion", status: "proposed" });

  /** Serves each tab's list based on the status filter. */
  function listByStatus() {
    return getListTasksMockHandler(({ request }) => {
      const status = new URL(request.url).searchParams.get("status");
      if (status === "in_review") return page([inReview]);
      if (status === "proposed") return page([proposed]);
      return page([]);
    });
  }

  function renderQueue(extraHandlers: RequestHandler[] = [], query = "") {
    return renderRoute(`/projects/${proj.id}/review${query}`, {
      handlers: [...extraHandlers, getGetProjectMockHandler(proj), listByStatus()],
    });
  }

  it("defaults to the in-review tab and lists waiting work", async () => {
    renderQueue();
    expect(await screen.findByText("Needs review")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "In review" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("Agent A")).toBeInTheDocument();
    expect(screen.getByText("1 attempt")).toBeInTheDocument();
  });

  it("switches tabs via the search param", async () => {
    renderQueue([], "?tab=proposals");
    expect(await screen.findByText("Agent suggestion")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Proposals" })).toHaveAttribute("aria-selected", "true");

    await userEvent.click(screen.getByRole("tab", { name: "In review" }));
    expect(await screen.findByText("Needs review")).toBeInTheDocument();
  });

  it("approves work in review", async () => {
    let approvedTaskId = "";
    renderQueue([
      getApproveTaskMockHandler(({ params }) => {
        approvedTaskId = String(params["taskId"]);
        return { ...inReview, status: "done" };
      }),
    ]);

    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    await waitFor(() => expect(approvedTaskId).toBe(inReview.id));
    expect(await screen.findByText(`Approved “${inReview.title}” — done.`)).toBeInTheDocument();
  });

  it("rejects work with a required reason", async () => {
    let body: unknown;
    renderQueue([
      getRejectTaskMockHandler(async ({ request }) => {
        body = await request.json();
        return { ...inReview, status: "ready" };
      }),
    ]);

    await userEvent.click(await screen.findByRole("button", { name: "Reject" }));
    const dialog = screen.getByRole("dialog");

    // Empty reason is refused client-side
    await userEvent.click(within(dialog).getByRole("button", { name: "Reject" }));
    expect(within(dialog).getByRole("textbox")).toHaveAttribute("aria-invalid", "true");

    await userEvent.type(within(dialog).getByRole("textbox"), "Doesn't handle pagination");
    await userEvent.click(within(dialog).getByRole("button", { name: "Reject" }));

    await waitFor(() => expect(body).toEqual({ reason: "Doesn't handle pagination" }));
    expect(
      await screen.findByText(`Rejected “${inReview.title}” — back to ready.`),
    ).toBeInTheDocument();
  });

  it("approves proposals", async () => {
    let approvedTaskId = "";
    renderQueue(
      [
        getApproveTaskMockHandler(({ params }) => {
          approvedTaskId = String(params["taskId"]);
          return { ...proposed, status: "approved" };
        }),
      ],
      "?tab=proposals",
    );

    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    await waitFor(() => expect(approvedTaskId).toBe(proposed.id));
    expect(await screen.findByText(`Approved proposal “${proposed.title}”.`)).toBeInTheDocument();
  });

  it("cancels proposals after confirmation", async () => {
    let cancelledTaskId = "";
    renderQueue(
      [
        getCancelTaskMockHandler(({ params }) => {
          cancelledTaskId = String(params["taskId"]);
          return { ...proposed, status: "cancelled" };
        }),
      ],
      "?tab=proposals",
    );

    await userEvent.click(await screen.findByRole("button", { name: "Cancel task" }));
    const dialog = screen.getByRole("dialog", { name: "Cancel proposal" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel task" }));
    await waitFor(() => expect(cancelledTaskId).toBe(proposed.id));
  });

  it("toasts 409 conflicts when someone else already handled the task", async () => {
    renderQueue([
      http.post("/api/v1/projects/:projectId/tasks/:taskId/approve", () =>
        problemResponse(409, "invalid-transition", {
          title: "Invalid transition",
          detail: "Task already approved elsewhere",
        }),
      ),
    ]);

    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Task already approved elsewhere");
  });

  it("shows empty states on both tabs", async () => {
    renderRoute(`/projects/${proj.id}/review`, {
      handlers: [getGetProjectMockHandler(proj), getListTasksMockHandler(page([]))],
    });
    expect(await screen.findByText("Nothing waiting on you")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("tab", { name: "Proposals" }));
    expect(await screen.findByText("No proposals waiting")).toBeInTheDocument();
  });
});

describe("Tab counts", () => {
  const proj = project({ name: "Tab counts" });

  it("shows first-page-honest counts on both tabs", async () => {
    renderRoute(`/projects/${proj.id}/review`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(({ request }) => {
          const status = new URL(request.url).searchParams.get("status");
          if (status === "in_review") return page([task({ status: "in_review" })]);
          if (status === "proposed") return page([task({ status: "proposed" })]);
          return page([]);
        }),
      ],
    });
    const inReviewTab = await screen.findByRole("tab", { name: "In review" });
    expect(await within(inReviewTab).findByText("1")).toBeInTheDocument();
    const proposalsTab = screen.getByRole("tab", { name: "Proposals" });
    expect(await within(proposalsTab).findByText("1")).toBeInTheDocument();
  });

  it("omits counts for empty queues", async () => {
    renderRoute(`/projects/${project().id}/review`, {
      handlers: [getListTasksMockHandler(page([]))],
    });
    const inReviewTab = await screen.findByRole("tab", { name: "In review" });
    expect(within(inReviewTab).queryByText(/\d/)).not.toBeInTheDocument();
  });
});
