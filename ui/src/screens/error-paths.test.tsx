/**
 * Cross-screen error-path and edge coverage: server failures, retry flows,
 * client-side guards, and pagination odds and ends that the per-screen happy
 * suites don't reach.
 */
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse, type RequestHandler } from "msw";
import { describe, expect, it, vi } from "vitest";
import { getListKnowledgeMockHandler } from "../api/generated/knowledge/knowledge.msw";
import { getGetProjectMockHandler } from "../api/generated/projects/projects.msw";
import { getListTaskRelationsMockHandler } from "../api/generated/relations/relations.msw";
import { getListTaskSessionsMockHandler } from "../api/generated/sessions/sessions.msw";
import { getGetTaskMockHandler, getListTasksMockHandler } from "../api/generated/tasks/tasks.msw";
import { knowledgeItem, page, project, session, task } from "../test/fixtures";
import { problemResponse } from "../test/msw";
import { renderRoute } from "../test/test-utils";

const proj = project({ name: "Alpha" });

describe("registry error paths", () => {
  it("shows a form-level error when project creation fails hard", async () => {
    renderRoute("/", {
      handlers: [
        http.post("/api/v1/projects", () =>
          problemResponse(500, "internal-error", {
            title: "Storage failure",
            detail: "Disk is full",
          }),
        ),
        getGetProjectMockHandler(proj),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Register project" }));
    await userEvent.type(screen.getByLabelText("Name"), "Doomed");
    await userEvent.click(screen.getByRole("button", { name: "Register" }));
    expect(await screen.findByText("Disk is full")).toBeInTheDocument();
  });

  it("shows import schema mismatches inside the dialog", async () => {
    renderRoute("/", {
      handlers: [
        http.post("/api/v1/projects/import", () =>
          problemResponse(422, "import-schema-mismatch", {
            title: "Import schema mismatch",
            detail: "Unsupported export version 9.0.0",
          }),
        ),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = screen.getByRole("dialog", { name: "Import project" });
    await userEvent.upload(
      screen.getByLabelText("Export file"),
      new File(['{"version":"9.0.0"}'], "old.json"),
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Import" }));
    expect(await screen.findByText("Unsupported export version 9.0.0")).toBeInTheDocument();
  });
});

describe("settings error paths", () => {
  it("toasts when export fails", async () => {
    renderRoute(`/projects/${proj.id}/settings`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.get("/api/v1/projects/:projectId/export", () =>
          problemResponse(500, "internal-error", {
            title: "Export failed",
            detail: "Could not assemble export",
          }),
        ),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Export project" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not assemble export");
  });

  it("recovers from a failed project load via retry", async () => {
    let failed = false;
    renderRoute(`/projects/${proj.id}/settings`, {
      handlers: [
        http.get(`/api/v1/projects/${proj.id}`, () => {
          if (!failed) {
            failed = true;
            return problemResponse(500, "internal-error", {
              title: "Flaky read",
            });
          }
          return undefined;
        }),
        getGetProjectMockHandler(proj),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
    expect(await screen.findByLabelText("Name")).toHaveValue("Alpha");
  });
});

describe("task list edge cases", () => {
  it("clears a filter back to All", async () => {
    const urls: string[] = [];
    renderRoute(`/projects/${proj.id}?status=done`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(({ request }) => {
          urls.push(request.url);
          return page([]);
        }),
      ],
    });
    const statusSelect = await screen.findByLabelText("Status");
    await userEvent.selectOptions(statusSelect, "");
    await waitFor(() => expect(urls.length).toBe(2));
    expect(new URL(urls.at(-1)!).searchParams.get("status")).toBeNull();
  });
});

describe("task form edge cases", () => {
  it("shows an error state when the task to edit cannot load", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${task().id}/edit`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.get("/api/v1/projects/:projectId/tasks/:taskId", () =>
          problemResponse(500, "internal-error", { title: "Cannot load task" }),
        ),
      ],
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Cannot load task");
  });

  it("cancel returns to the task list without writing", async () => {
    renderRoute(`/projects/${proj.id}/tasks/new`, {
      handlers: [getGetProjectMockHandler(proj), getListTasksMockHandler(page([]))],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
  });

  it("shows a form-level error when creation fails hard", async () => {
    renderRoute(`/projects/${proj.id}/tasks/new`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.post("/api/v1/projects/:projectId/tasks", () =>
          problemResponse(500, "internal-error", {
            title: "Storage failure",
            detail: "Write failed",
          }),
        ),
      ],
    });
    await userEvent.type(await screen.findByLabelText("Title"), "Doomed");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    expect(await screen.findByText("Write failed")).toBeInTheDocument();
  });

  it("formats valid metadata and leaves invalid metadata untouched", async () => {
    renderRoute(`/projects/${proj.id}/tasks/new`, {
      handlers: [getGetProjectMockHandler(proj)],
    });
    const metadata = await screen.findByLabelText("Metadata");

    await userEvent.type(metadata, '{{"a":1}');
    await userEvent.click(screen.getByRole("button", { name: "Format" }));
    expect(metadata).toHaveValue('{\n  "a": 1\n}');

    await userEvent.clear(metadata);
    await userEvent.type(metadata, "{{oops");
    await userEvent.click(screen.getByRole("button", { name: "Format" }));
    expect(metadata).toHaveValue("{oops");
  });
});

describe("task detail panel failures", () => {
  const main = task({ title: "Panel test", status: "ready" });

  function handlers(overrides: RequestHandler[]) {
    return [
      ...overrides,
      getGetProjectMockHandler(proj),
      getGetTaskMockHandler(main),
      getListTaskRelationsMockHandler({ items: [] }),
      getListTaskSessionsMockHandler(page([])),
      getListKnowledgeMockHandler(page([])),
    ];
  }

  it("degrades the relations panel independently", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([
        http.get("/api/v1/projects/:projectId/tasks/:taskId/relations", () =>
          problemResponse(500, "internal-error", { title: "Relations broke" }),
        ),
      ]),
    });
    expect(await screen.findByRole("heading", { name: "Panel test" })).toBeInTheDocument();
    expect(await screen.findByText("Relations broke")).toBeInTheDocument();
  });

  it("degrades the sessions panel independently", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([
        http.get("/api/v1/projects/:projectId/tasks/:taskId/sessions", () =>
          problemResponse(500, "internal-error", { title: "Sessions broke" }),
        ),
      ]),
    });
    expect(await screen.findByRole("heading", { name: "Panel test" })).toBeInTheDocument();
    expect(await screen.findByText("Sessions broke")).toBeInTheDocument();
  });

  it("degrades the knowledge panel independently", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([
        http.get("/api/v1/projects/:projectId/knowledge", () =>
          problemResponse(500, "internal-error", { title: "Knowledge broke" }),
        ),
      ]),
    });
    expect(await screen.findByRole("heading", { name: "Panel test" })).toBeInTheDocument();
    expect(await screen.findByText("Knowledge broke")).toBeInTheDocument();
  });

  it("pages through older sessions", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([
        getListTaskSessionsMockHandler(({ request }) => {
          const cursor = new URL(request.url).searchParams.get("cursor");
          return cursor === "older"
            ? page([session({ summary: "The first attempt" })])
            : page([session({ summary: "The latest attempt" })], "older");
        }),
      ]),
    });
    expect(await screen.findByText("The latest attempt")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("The first attempt")).toBeInTheDocument();
  });

  it("requires picking a task before adding a relation", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([getListTasksMockHandler(page([]))]),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Add relation" }));
    const dialog = screen.getByRole("dialog", { name: "Add relation" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Add relation" }));
    expect(await within(dialog).findByText("Pick a task first.")).toBeInTheDocument();
  });

  it("validates knowledge client-side and surfaces server failures", async () => {
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: handlers([
        http.post("/api/v1/projects/:projectId/knowledge", () =>
          problemResponse(500, "internal-error", {
            title: "Storage failure",
            detail: "Knowledge write failed",
          }),
        ),
      ]),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Add knowledge" }));
    const dialog = screen.getByRole("dialog", { name: "Add knowledge" });

    // Empty title/content rejected client-side
    await userEvent.click(within(dialog).getByRole("button", { name: "Add knowledge" }));
    expect(within(dialog).getByLabelText("Title")).toHaveAttribute("aria-invalid", "true");

    await userEvent.type(within(dialog).getByLabelText("Title"), "T");
    await userEvent.type(within(dialog).getByLabelText("Content"), "C");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add knowledge" }));
    expect(await within(dialog).findByText("Knowledge write failed")).toBeInTheDocument();
  });
});

describe("review queue edge cases", () => {
  const first = task({ title: "Review one", status: "in_review" });
  const second = task({ title: "Review two", status: "in_review" });

  it("pages through the review queue", async () => {
    renderRoute(`/projects/${proj.id}/review`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(({ request }) => {
          const cursor = new URL(request.url).searchParams.get("cursor");
          return cursor === "next" ? page([second]) : page([first], "next");
        }),
      ],
    });
    expect(await screen.findByText("Review one")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Review two")).toBeInTheDocument();
  });

  it("toasts and refetches when a proposal approval conflicts", async () => {
    const listCalls = vi.fn();
    renderRoute(`/projects/${proj.id}/review?tab=proposals`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.post("/api/v1/projects/:projectId/tasks/:taskId/approve", () =>
          problemResponse(409, "invalid-transition", {
            title: "Invalid transition",
            detail: "Proposal already handled",
          }),
        ),
        getListTasksMockHandler(() => {
          listCalls();
          return page([task({ title: "A proposal", status: "proposed" })]);
        }),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Proposal already handled");
    await waitFor(() => expect(listCalls.mock.calls.length).toBeGreaterThan(1));
  });

  it("keeps the reject dialog open when rejection conflicts", async () => {
    renderRoute(`/projects/${proj.id}/review`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.post("/api/v1/projects/:projectId/tasks/:taskId/reject", () =>
          problemResponse(409, "invalid-transition", {
            title: "Invalid transition",
            detail: "No longer in review",
          }),
        ),
        getListTasksMockHandler(page([first])),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Reject" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "Nope");
    await userEvent.click(within(dialog).getByRole("button", { name: "Reject" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No longer in review");
  });

  it("dismisses the proposal-cancel dialog without cancelling", async () => {
    let cancelled = false;
    renderRoute(`/projects/${proj.id}/review?tab=proposals`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.post("/api/v1/projects/:projectId/tasks/:taskId/cancel", () => {
          cancelled = true;
          return problemResponse(500, "internal-error", {});
        }),
        getListTasksMockHandler(page([task({ title: "Keep me", status: "proposed" })])),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Cancel task" }));
    const dialog = screen.getByRole("dialog", { name: "Cancel proposal" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Cancel proposal" })).not.toBeInTheDocument();
    expect(cancelled).toBe(false);
  });
});

describe("task detail action edge cases", () => {
  const main = task({ title: "Actionable", status: "ready" });

  function base(overrides: RequestHandler[], detailTask = main) {
    return {
      handlers: [
        ...overrides,
        getGetProjectMockHandler(proj),
        getGetTaskMockHandler(detailTask),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
      ],
    };
  }

  it("approves a proposal from the detail screen", async () => {
    const proposal = task({ title: "Proposed work", status: "proposed" });
    renderRoute(
      `/projects/${proj.id}/tasks/${proposal.id}`,
      base(
        [
          http.post("/api/v1/projects/:projectId/tasks/:taskId/approve", () =>
            HttpResponse.json({ ...proposal, status: "approved" }),
          ),
        ],
        proposal,
      ),
    );
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Proposal approved.")).toBeInTheDocument();
  });

  it("dismisses every action dialog without firing the action", async () => {
    const inReview = task({ title: "Dismissable", status: "in_review" });
    renderRoute(`/projects/${proj.id}/tasks/${inReview.id}`, base([], inReview));

    // Reject → cancel
    await userEvent.click(await screen.findByRole("button", { name: "Reject" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Block → cancel
    await userEvent.click(screen.getByRole("button", { name: "Block" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Cancel task → dismiss
    await userEvent.click(screen.getByRole("button", { name: "Cancel task" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Delete → dismiss
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("recovers from a failed task load via retry", async () => {
    let failed = false;
    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: [
        http.get("/api/v1/projects/:projectId/tasks/:taskId", () => {
          if (!failed) {
            failed = true;
            return problemResponse(500, "internal-error", { title: "Flaky" });
          }
          return undefined;
        }),
        getGetProjectMockHandler(proj),
        getGetTaskMockHandler(main),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("heading", { name: "Actionable" })).toBeInTheDocument();
  });

  it("pages through task knowledge", async () => {
    renderRoute(
      `/projects/${proj.id}/tasks/${main.id}`,
      base([
        getListKnowledgeMockHandler(({ request }) => {
          const cursor = new URL(request.url).searchParams.get("cursor");
          return cursor === "more"
            ? page([knowledgeItem({ title: "Older wisdom" })])
            : page([knowledgeItem({ title: "Fresh wisdom" })], "more");
        }),
      ]),
    );
    expect(await screen.findByText("Fresh wisdom")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Older wisdom")).toBeInTheDocument();
  });
});

describe("more form and settings edges", () => {
  it("edit-mode cancel returns to the task detail", async () => {
    const existing = task({ title: "Cancelled edit", status: "ready" });
    renderRoute(`/projects/${proj.id}/tasks/${existing.id}/edit`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getGetTaskMockHandler(existing),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
      ],
    });
    await waitFor(async () =>
      expect(await screen.findByLabelText("Title")).toHaveValue("Cancelled edit"),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByRole("heading", { name: "Cancelled edit" })).toBeInTheDocument();
  });

  it("shows a form-level error when saving settings fails hard", async () => {
    renderRoute(`/projects/${proj.id}/settings`, {
      handlers: [
        http.patch("/api/v1/projects/:projectId", () =>
          problemResponse(500, "internal-error", {
            title: "Storage failure",
            detail: "Settings write failed",
          }),
        ),
        getGetProjectMockHandler(proj),
      ],
    });
    await screen.findByDisplayValue("Alpha");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByText("Settings write failed")).toBeInTheDocument();
  });
});
