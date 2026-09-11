import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getExportProjectMockHandler } from "../../api/generated/export-import/export-import.msw";
import {
  getDeleteProjectMockHandler,
  getGetProjectMockHandler,
  getListProjectsMockHandler,
  getUpdateProjectMockHandler,
} from "../../api/generated/projects/projects.msw";
import { page, project } from "../../test/fixtures";
import { problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("Project settings", () => {
  afterEach(() => vi.unstubAllGlobals());

  const existing = project({
    name: "Alpha",
    description: "First project",
    settings: { review_gate: true },
  });

  function renderSettings(extra: Parameters<typeof renderRoute>[1] = {}) {
    return renderRoute(`/projects/${existing.id}/settings`, {
      handlers: [getGetProjectMockHandler(existing), ...(extra.handlers ?? [])],
    });
  }

  it("prefills the form from the project", async () => {
    renderSettings();
    expect(await screen.findByLabelText("Name")).toHaveValue("Alpha");
    expect(screen.getByLabelText("Description")).toHaveValue("First project");
    expect(screen.getByRole("checkbox")).toBeChecked();
  });

  it("saves the review-gate toggle via PATCH", async () => {
    let body: unknown;
    renderSettings({
      handlers: [
        getUpdateProjectMockHandler(async ({ request }) => {
          body = await request.json();
          return {
            ...existing,
            settings: { review_gate: false },
          };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));

    expect(await screen.findByText("Project settings saved.")).toBeInTheDocument();
    expect(body).toEqual({
      name: "Alpha",
      description: "First project",
      settings: { review_gate: false },
    });
  });

  it("maps server 422 errors onto fields", async () => {
    renderSettings({
      handlers: [
        http.patch(`/api/v1/projects/${existing.id}`, () =>
          problemResponse(422, "validation-error", {
            errors: [{ field: "/name", message: "name already taken" }],
          }),
        ),
      ],
    });
    const nameInput = await screen.findByLabelText("Name");
    await userEvent.clear(nameInput);
    await userEvent.type(nameInput, "Taken");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByText("name already taken")).toBeInTheDocument();
  });

  it("blocks empty names client-side", async () => {
    renderSettings();
    const nameInput = await screen.findByLabelText("Name");
    await userEvent.clear(nameInput);
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(nameInput).toHaveAttribute("aria-invalid", "true");
  });

  it("exports the project as a JSON download", async () => {
    // jsdom has no createObjectURL — augment the real URL class (replacing it
    // wholesale would break `new URL()` inside fetch/MSW).
    const createObjectURL = vi.fn(() => "blob:mock");
    const revokeObjectURL = vi.fn();
    Object.assign(URL, { createObjectURL, revokeObjectURL });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    renderSettings({
      handlers: [
        getExportProjectMockHandler({
          version: "1.0.0",
          project: existing,
          tasks: [],
          relations: [],
          sessions: [],
          knowledge: [],
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Export project" }));
    await waitFor(() => expect(click).toHaveBeenCalledOnce());
    expect(createObjectURL).toHaveBeenCalledOnce();
    click.mockRestore();
    delete (URL as { createObjectURL?: unknown }).createObjectURL;
    delete (URL as { revokeObjectURL?: unknown }).revokeObjectURL;
  });

  it("deletes the project after confirmation and returns to the registry", async () => {
    let deleted = false;
    renderSettings({
      handlers: [
        getDeleteProjectMockHandler(() => {
          deleted = true;
        }),
        getListProjectsMockHandler(page([])),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Delete project" }));
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    await userEvent.click(screen.getAllByRole("button", { name: "Delete project" })[1]!);

    await waitFor(() => expect(deleted).toBe(true));
    expect(dialog).not.toBeInTheDocument();
    expect(await screen.findByText("Register your first project")).toBeInTheDocument();
  });

  it("shows an error state when the project fails to load", async () => {
    renderRoute(`/projects/${existing.id}/settings`, {
      handlers: [
        http.get(`/api/v1/projects/${existing.id}`, () =>
          problemResponse(404, "not-found", { title: "Project not found" }),
        ),
      ],
    });
    expect((await screen.findAllByText("Project not found")).length).toBeGreaterThan(0);
  });
});

describe("Settings state isolation", () => {
  it("resets form state when navigating between cached projects", async () => {
    const alpha = project({ name: "Alpha proj", description: "a" });
    const beta = project({ name: "Beta proj", description: "b" });
    const byId = new Map([
      [alpha.id, alpha],
      [beta.id, beta],
    ]);
    const { router } = renderRoute(`/projects/${beta.id}/settings`, {
      handlers: [
        getGetProjectMockHandler(({ params }) => byId.get(String(params["projectId"])) ?? alpha),
      ],
    });

    // Beta loads (and is now cached), then we visit Alpha…
    await waitFor(async () =>
      expect(await screen.findByLabelText("Name")).toHaveValue("Beta proj"),
    );
    await act(async () => {
      await router.navigate(`/projects/${alpha.id}/settings`);
    });
    await waitFor(() => expect(screen.getByLabelText("Name")).toHaveValue("Alpha proj"));

    // …and back to Beta, which is cached (no loading remount). Without the
    // project key, the form would still hold Alpha's values here.
    await act(async () => {
      await router.navigate(`/projects/${beta.id}/settings`);
    });
    await waitFor(() => expect(screen.getByLabelText("Name")).toHaveValue("Beta proj"));
  });
});
