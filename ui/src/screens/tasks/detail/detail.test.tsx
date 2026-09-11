import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import type { Task } from "../../../api/generated/model";
import {
  getCreateKnowledgeMockHandler,
  getDeleteKnowledgeMockHandler,
  getListKnowledgeMockHandler,
} from "../../../api/generated/knowledge/knowledge.msw";
import { getGetProjectMockHandler } from "../../../api/generated/projects/projects.msw";
import {
  getCreateTaskRelationMockHandler,
  getDeleteTaskRelationMockHandler,
  getListTaskRelationsMockHandler,
} from "../../../api/generated/relations/relations.msw";
import { getListTaskSessionsMockHandler } from "../../../api/generated/sessions/sessions.msw";
import {
  getApproveTaskMockHandler,
  getBlockTaskMockHandler,
  getCancelTaskMockHandler,
  getDeleteTaskMockHandler,
  getGetTaskMockHandler,
  getListTasksMockHandler,
  getRejectTaskMockHandler,
  getUnblockTaskMockHandler,
} from "../../../api/generated/tasks/tasks.msw";
import {
  identity,
  knowledgeItem,
  page,
  project,
  relation,
  session,
  task,
} from "../../../test/fixtures";
import { problemResponse } from "../../../test/msw";
import { renderRoute } from "../../../test/test-utils";

describe("Task detail", () => {
  const proj = project({ name: "Alpha" });

  function taskByIdHandler(tasks: Task[]) {
    const byId = new Map(tasks.map((t) => [t.id, t]));
    return getGetTaskMockHandler(({ params }) => {
      const found = byId.get(String(params["taskId"]));
      return found ?? tasks[0]!;
    });
  }

  function renderDetail(main: Task, extra: Parameters<typeof renderRoute>[1] = {}) {
    // MSW resolves against the FIRST matching handler, so test-specific
    // handlers go first and these defaults only catch what's left over.
    return renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: [
        ...(extra.handlers ?? []),
        getGetProjectMockHandler(proj),
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
      ],
    });
  }

  it("aggregates fields, relations, sessions, attempts, and knowledge", async () => {
    const main = task({
      title: "Main task",
      description: "Build the thing",
      status: "in_progress",
      assignee: identity({ label: "Agent A" }),
      attempt_count: 3,
      graph_role: ["milestone"],
      metadata: { branch: "s7-ui" },
    });
    const parent = task({ title: "Parent task" });
    const sub = task({ title: "Subtask A" });
    const dep = task({ title: "Prerequisite" });
    const blocker = task({ title: "Downstream" });

    renderRoute(`/projects/${proj.id}/tasks/${main.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        taskByIdHandler([main, parent, sub, dep, blocker]),
        getListTaskRelationsMockHandler({
          items: [
            relation({
              type: "decomposition",
              source_task_id: parent.id,
              target_task_id: main.id,
            }),
            relation({
              type: "decomposition",
              source_task_id: main.id,
              target_task_id: sub.id,
            }),
            relation({
              type: "depends_on",
              source_task_id: main.id,
              target_task_id: dep.id,
            }),
            relation({
              type: "depends_on",
              source_task_id: blocker.id,
              target_task_id: main.id,
            }),
          ],
        }),
        getListTaskSessionsMockHandler(
          page([
            session({
              summary: "First try",
              outcome: "failed",
              failure_reason: "Tests kept failing",
              identity: identity({ label: "Agent A" }),
              decisions: ["Chose approach X"],
              artifacts: ["https://example.com/pr/1"],
            }),
            session({
              summary: "Second try worked",
              outcome: "succeeded",
              knowledge_items: [knowledgeItem({ title: "Learned a trick" })],
            }),
          ]),
        ),
        getListKnowledgeMockHandler(
          page([
            knowledgeItem({ title: "Design note", content: "Keep it lean" }),
            knowledgeItem({
              title: "The design doc",
              type: "link",
              content: "https://example.com/design-doc",
            }),
          ]),
        ),
      ],
    });

    // Core fields
    expect(await screen.findByRole("heading", { name: "Main task" })).toBeInTheDocument();
    expect(screen.getByText("In progress")).toHaveAttribute("data-status", "in_progress");
    expect(screen.getByText("Build the thing")).toBeInTheDocument();
    expect(screen.getByText("milestone")).toBeInTheDocument();
    expect(screen.getByText(/"branch": "s7-ui"/)).toBeInTheDocument();

    // Relations grouped in all four directions with resolved titles
    expect(await screen.findByText("Parent task")).toBeInTheDocument();
    expect(screen.getByText("Subtask A")).toBeInTheDocument();
    expect(screen.getByText("Prerequisite")).toBeInTheDocument();
    expect(screen.getByText("Downstream")).toBeInTheDocument();
    expect(screen.getByText("Parent")).toBeInTheDocument();
    expect(screen.getByText("Subtasks")).toBeInTheDocument();
    expect(screen.getByText("Depends on")).toBeInTheDocument();
    expect(screen.getByText("Depended on by")).toBeInTheDocument();

    // Sessions timeline with failure surfaced
    expect(screen.getByText("First try")).toBeInTheDocument();
    expect(screen.getByText("Tests kept failing")).toBeInTheDocument();
    expect(screen.getByText("Chose approach X")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "https://example.com/pr/1" })).toBeInTheDocument();
    expect(screen.getByText("Second try worked")).toBeInTheDocument();
    expect(screen.getByText(/Learned a trick/)).toBeInTheDocument();

    // Attempt history: 3 attempts, 1 failed session loaded
    expect(screen.getByText("3 attempts, 1 failed")).toBeInTheDocument();

    // Knowledge panel
    expect(screen.getByText("Design note")).toBeInTheDocument();
    expect(screen.getByText("Keep it lean")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "https://example.com/design-doc" }),
    ).toBeInTheDocument();
  });

  it("shows a full error state when the task is missing", async () => {
    renderRoute(`/projects/${proj.id}/tasks/00000000-0000-4000-8000-999999999999`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.get("/api/v1/projects/:projectId/tasks/:taskId", () =>
          problemResponse(404, "not-found", { title: "Task not found" }),
        ),
      ],
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Task not found");
  });

  it("blocks a task with a required reason", async () => {
    const main = task({ title: "Blockable", status: "ready" });
    let body: unknown;
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getBlockTaskMockHandler(async ({ request }) => {
          body = await request.json();
          return { ...main, status: "blocked" };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Block" }));
    const dialog = screen.getByRole("dialog", { name: "Block task" });

    // Reason is required
    await userEvent.click(within(dialog).getByRole("button", { name: "Block" }));
    expect(within(dialog).getByRole("textbox")).toHaveAttribute("aria-invalid", "true");

    await userEvent.type(within(dialog).getByRole("textbox"), "Waiting on credentials");
    await userEvent.click(within(dialog).getByRole("button", { name: "Block" }));

    await waitFor(() => expect(body).toEqual({ reason: "Waiting on credentials" }));
    expect(await screen.findByText("Task blocked.")).toBeInTheDocument();
  });

  it("unblocks a blocked task", async () => {
    const main = task({ title: "Stuck", status: "blocked" });
    let called = false;
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getUnblockTaskMockHandler(() => {
          called = true;
          return { ...main, status: "ready" };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Unblock" }));
    await waitFor(() => expect(called).toBe(true));
    expect(await screen.findByText("Task unblocked.")).toBeInTheDocument();
  });

  it("cancels a task after confirmation", async () => {
    const main = task({ title: "Doomed", status: "ready" });
    let called = false;
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getCancelTaskMockHandler(() => {
          called = true;
          return { ...main, status: "cancelled" };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Cancel task" }));
    const dialog = screen.getByRole("dialog", { name: "Cancel task" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel task" }));
    await waitFor(() => expect(called).toBe(true));
    expect(await screen.findByText("Task cancelled.")).toBeInTheDocument();
  });

  it("surfaces 409 invalid-transition problems as a toast", async () => {
    const main = task({ title: "Racy", status: "in_review" });
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        http.post("/api/v1/projects/:projectId/tasks/:taskId/approve", () =>
          problemResponse(409, "invalid-transition", {
            title: "Invalid transition",
            detail: "Task is no longer in review",
          }),
        ),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Task is no longer in review");
  });

  it("approves and rejects work in review from the detail screen", async () => {
    const main = task({ title: "Reviewable", status: "in_review" });
    let rejectBody: unknown;
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getApproveTaskMockHandler({ ...main, status: "done" }),
        getRejectTaskMockHandler(async ({ request }) => {
          rejectBody = await request.json();
          return { ...main, status: "ready" };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Reject" }));
    const dialog = screen.getByRole("dialog", { name: "Reject work" });
    await userEvent.type(within(dialog).getByRole("textbox"), "Missing tests");
    await userEvent.click(within(dialog).getByRole("button", { name: "Reject" }));
    await waitFor(() => expect(rejectBody).toEqual({ reason: "Missing tests" }));

    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Work approved — task is done.")).toBeInTheDocument();
  });

  it("deletes the task and returns to the list", async () => {
    const main = task({ title: "Disposable", status: "done" });
    let deleted = false;
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getDeleteTaskMockHandler(() => {
          deleted = true;
        }),
        getListTasksMockHandler(page([])),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));
    const dialog = screen.getByRole("dialog", { name: "Delete task" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleted).toBe(true));
    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
  });

  it("adds a depends_on relation from the picker", async () => {
    const main = task({ title: "Needs prereq", status: "approved" });
    const other = task({ title: "The prereq" });
    let body: unknown;
    let postedTaskId = "";
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main, other]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getListTasksMockHandler(page([main, other])),
        getCreateTaskRelationMockHandler(async ({ request, params }) => {
          body = await request.json();
          postedTaskId = String(params["taskId"]);
          return relation({
            type: "depends_on",
            source_task_id: main.id,
            target_task_id: other.id,
          });
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Add relation" }));
    const dialog = screen.getByRole("dialog", { name: "Add relation" });
    await userEvent.selectOptions(within(dialog).getByLabelText("Kind"), "depends_on");
    const picker = within(dialog).getByLabelText("Task");
    await waitFor(() => expect(within(picker).getByText("The prereq")).toBeInTheDocument());
    await userEvent.selectOptions(picker, other.id);
    await userEvent.click(within(dialog).getByRole("button", { name: "Add relation" }));

    await waitFor(() => expect(body).toEqual({ type: "depends_on", target_task_id: other.id }));
    expect(postedTaskId).toBe(main.id);
    expect(await screen.findByText("Relation added.")).toBeInTheDocument();
  });

  it("posts parent relations onto the selected task", async () => {
    const main = task({ title: "Child" });
    const parent = task({ title: "The parent" });
    let body: unknown;
    let postedTaskId = "";
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main, parent]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getListTasksMockHandler(page([main, parent])),
        getCreateTaskRelationMockHandler(async ({ request, params }) => {
          body = await request.json();
          postedTaskId = String(params["taskId"]);
          return relation({
            type: "decomposition",
            source_task_id: parent.id,
            target_task_id: main.id,
          });
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Add relation" }));
    const dialog = screen.getByRole("dialog", { name: "Add relation" });
    await userEvent.selectOptions(within(dialog).getByLabelText("Kind"), "parent");
    const picker = within(dialog).getByLabelText("Task");
    await waitFor(() => expect(within(picker).getByText("The parent")).toBeInTheDocument());
    await userEvent.selectOptions(picker, parent.id);
    await userEvent.click(within(dialog).getByRole("button", { name: "Add relation" }));

    // decomposition source is the parent → POST on the parent, this task as target
    await waitFor(() =>
      expect(body).toEqual({
        type: "decomposition",
        target_task_id: main.id,
      }),
    );
    expect(postedTaskId).toBe(parent.id);
  });

  it("renders dependency-cycle conflicts inside the dialog", async () => {
    const main = task({ title: "Cyclic" });
    const other = task({ title: "Other" });
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main, other]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getListTasksMockHandler(page([main, other])),
        http.post("/api/v1/projects/:projectId/tasks/:taskId/relations", () =>
          problemResponse(409, "dependency-cycle", {
            title: "Dependency cycle",
            detail: "This relation would create a cycle",
          }),
        ),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Add relation" }));
    const dialog = screen.getByRole("dialog", { name: "Add relation" });
    const picker = within(dialog).getByLabelText("Task");
    await waitFor(() => expect(within(picker).getByText("Other")).toBeInTheDocument());
    await userEvent.selectOptions(picker, other.id);
    await userEvent.click(within(dialog).getByRole("button", { name: "Add relation" }));

    expect(
      await within(dialog).findByText("This relation would create a cycle"),
    ).toBeInTheDocument();
  });

  it("removes a relation after confirmation", async () => {
    const main = task({ title: "Linked" });
    const dep = task({ title: "Dependency" });
    let deletedRelationId = "";
    const rel = relation({
      type: "depends_on",
      source_task_id: main.id,
      target_task_id: dep.id,
    });
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main, dep]),
        getListTaskRelationsMockHandler({ items: [rel] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([])),
        getDeleteTaskRelationMockHandler(({ params }) => {
          deletedRelationId = String(params["relationId"]);
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Remove relation" }));
    const dialog = screen.getByRole("dialog", { name: "Remove relation" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(deletedRelationId).toBe(rel.id));
  });

  it("adds and deletes task-scoped knowledge", async () => {
    const main = task({ title: "Knowledgeable" });
    const existing = knowledgeItem({ title: "Old note" });
    let createBody: unknown;
    let deletedId = "";
    renderDetail(main, {
      handlers: [
        taskByIdHandler([main]),
        getListTaskRelationsMockHandler({ items: [] }),
        getListTaskSessionsMockHandler(page([])),
        getListKnowledgeMockHandler(page([existing])),
        getCreateKnowledgeMockHandler(async ({ request }) => {
          createBody = await request.json();
          return knowledgeItem({ title: "Fresh insight" });
        }),
        getDeleteKnowledgeMockHandler(({ params }) => {
          deletedId = String(params["knowledgeId"]);
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Add knowledge" }));
    const dialog = screen.getByRole("dialog", { name: "Add knowledge" });
    await userEvent.type(within(dialog).getByLabelText("Title"), "Fresh insight");
    await userEvent.type(within(dialog).getByLabelText("Content"), "Never trust defaults");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add knowledge" }));

    await waitFor(() =>
      expect(createBody).toEqual({
        type: "note",
        title: "Fresh insight",
        content: "Never trust defaults",
        scope: "task",
        task_id: main.id,
      }),
    );

    await userEvent.click(screen.getByRole("button", { name: "Delete Old note" }));
    const confirm = screen.getByRole("dialog", {
      name: "Delete knowledge item",
    });
    await userEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deletedId).toBe(existing.id));
  });
});
