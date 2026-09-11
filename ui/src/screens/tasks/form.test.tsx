import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import {
  getCreateTaskMockHandler,
  getGetTaskMockHandler,
  getUpdateTaskMockHandler,
} from "../../api/generated/tasks/tasks.msw";
import { project, task } from "../../test/fixtures";
import { problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";
import { validateMetadata } from "./MetadataEditor";

describe("validateMetadata", () => {
  it("treats empty input as an empty object", () => {
    expect(validateMetadata("  ")).toEqual({ ok: true, value: {} });
  });

  it("rejects invalid JSON", () => {
    expect(validateMetadata("{nope")).toEqual({
      ok: false,
      error: "Metadata must be valid JSON.",
    });
  });

  it("rejects non-object JSON", () => {
    for (const bad of ["[1,2]", "null", '"text"', "42"]) {
      expect(validateMetadata(bad).ok).toBe(false);
    }
  });

  it("rejects objects with more than 200 keys", () => {
    const wide = Object.fromEntries(Array.from({ length: 201 }, (_, i) => [`k${i}`, i]));
    expect(validateMetadata(JSON.stringify(wide)).ok).toBe(false);
  });

  it("accepts a plain object", () => {
    expect(validateMetadata('{"branch": "main"}')).toEqual({
      ok: true,
      value: { branch: "main" },
    });
  });
});

describe("Task form", () => {
  const proj = project({ name: "Alpha" });

  function renderForm(extra: Parameters<typeof renderRoute>[1] = {}, path = "/tasks/new") {
    return renderRoute(`/projects/${proj.id}${path}`, {
      handlers: [getGetProjectMockHandler(proj), ...(extra.handlers ?? [])],
    });
  }

  it("creates a task as approved by default and navigates to it", async () => {
    const created = task({ title: "Ship the UI", status: "approved" });
    let body: unknown;
    renderForm({
      handlers: [
        getCreateTaskMockHandler(async ({ request }) => {
          body = await request.json();
          return created;
        }),
        getGetTaskMockHandler(created),
      ],
    });

    await userEvent.type(await screen.findByLabelText("Title"), "Ship the UI");
    await userEvent.type(screen.getByLabelText("Description"), "All of S7");
    await userEvent.selectOptions(screen.getByLabelText("Type"), "refactor");
    await userEvent.type(screen.getByLabelText("Metadata"), '{{"branch": "s7-ui"}');
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() =>
      expect(body).toEqual({
        title: "Ship the UI",
        description: "All of S7",
        type: "refactor",
        status: "approved",
        metadata: { branch: "s7-ui" },
      }),
    );
    // Landed on the task detail screen.
    expect(await screen.findByRole("heading", { name: "Ship the UI" })).toBeInTheDocument();
  });

  it("creates a proposal when the proposed radio is picked", async () => {
    const created = task({ title: "Suggested", status: "proposed" });
    let body: unknown;
    renderForm({
      handlers: [
        getCreateTaskMockHandler(async ({ request }) => {
          body = await request.json();
          return created;
        }),
        getGetTaskMockHandler(created),
      ],
    });

    await userEvent.type(await screen.findByLabelText("Title"), "Suggested");
    await userEvent.click(screen.getByRole("radio", { name: /Proposed — needs human approval/ }));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() => expect(body).toMatchObject({ title: "Suggested", status: "proposed" }));
  });

  it("blocks submission when the title is empty", async () => {
    renderForm();
    await userEvent.click(await screen.findByRole("button", { name: "Create task" }));
    expect(screen.getByLabelText("Title")).toHaveAttribute("aria-invalid", "true");
  });

  it("rejects invalid metadata JSON before submitting", async () => {
    renderForm();
    await userEvent.type(await screen.findByLabelText("Title"), "T");
    await userEvent.type(screen.getByLabelText("Metadata"), "{{broken");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    expect(await screen.findByText("Metadata must be valid JSON.")).toBeInTheDocument();
  });

  it("maps server 422 errors onto fields", async () => {
    renderForm({
      handlers: [
        http.post("/api/v1/projects/:projectId/tasks", () =>
          problemResponse(422, "validation-error", {
            errors: [{ field: "/title", message: "title is too bland" }],
          }),
        ),
      ],
    });
    await userEvent.type(await screen.findByLabelText("Title"), "Meh");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    expect(await screen.findByText("title is too bland")).toBeInTheDocument();
  });

  it("edits a task, PATCHing only changed fields and never status", async () => {
    const existing = task({
      title: "Old title",
      description: "Same description",
      type: "code",
      status: "ready",
      metadata: { keep: true },
    });
    let body: unknown;
    renderForm(
      {
        handlers: [
          getGetTaskMockHandler(existing),
          getUpdateTaskMockHandler(async ({ request }) => {
            body = await request.json();
            return { ...existing, title: "New title" };
          }),
        ],
      },
      `/tasks/${existing.id}/edit`,
    );

    const title = await screen.findByLabelText("Title");
    await waitFor(() => expect(title).toHaveValue("Old title"));
    // Status choice is a create-only concept.
    expect(screen.queryByText("Initial status")).not.toBeInTheDocument();

    await userEvent.clear(title);
    await userEvent.type(title, "New title");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(body).toEqual({ title: "New title" }));
  });

  it("skips the PATCH entirely when nothing changed", async () => {
    const existing = task({ title: "Untouched", status: "ready" });
    let patched = false;
    renderForm(
      {
        handlers: [
          getGetTaskMockHandler(existing),
          getUpdateTaskMockHandler(() => {
            patched = true;
            return existing;
          }),
        ],
      },
      `/tasks/${existing.id}/edit`,
    );

    const title = await screen.findByLabelText("Title");
    await waitFor(() => expect(title).toHaveValue("Untouched"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    // Back on the detail screen without a write.
    expect(await screen.findByRole("heading", { name: "Untouched" })).toBeInTheDocument();
    expect(patched).toBe(false);
  });
});
