import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getImportProjectMockHandler } from "../../api/generated/export-import/export-import.msw";
import {
  getCreateProjectMockHandler,
  getListProjectsMockHandler,
} from "../../api/generated/projects/projects.msw";
import { page, project, uuid } from "../../test/fixtures";
import { problemResponse, server } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

const PROJECTS_URL = "/api/v1/projects";

describe("Project registry", () => {
  it("lists projects with review gate and link", async () => {
    const p1 = project({ name: "Alpha", settings: { review_gate: true } });
    const p2 = project({
      name: "Beta",
      description: "Second project",
      settings: { review_gate: false },
    });
    renderRoute("/", {
      handlers: [getListProjectsMockHandler(page([p1, p2]))],
    });

    expect(await screen.findByRole("link", { name: "Alpha" })).toHaveAttribute(
      "href",
      `/projects/${p1.id}`,
    );
    expect(screen.getByText("Second project")).toBeInTheDocument();
    expect(screen.getByText("On")).toBeInTheDocument();
    expect(screen.getByText("Off")).toBeInTheDocument();
  });

  it("shows the empty state", async () => {
    renderRoute("/", { handlers: [getListProjectsMockHandler(page([]))] });
    expect(await screen.findByText("Register your first project")).toBeInTheDocument();
  });

  it("shows an error state with retry on 500", async () => {
    let calls = 0;
    server.use(
      http.get(PROJECTS_URL, () => {
        calls += 1;
        return problemResponse(500, "internal-error", {
          title: "Storage failure",
        });
      }),
    );
    renderRoute("/");
    expect(await screen.findByRole("alert")).toHaveTextContent("Storage failure");

    server.use(getListProjectsMockHandler(page([project({ name: "Recovered" })])));
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Recovered")).toBeInTheDocument();
    expect(calls).toBe(1);
  });

  it("pages through the cursor", async () => {
    const first = project({ name: "Page one" });
    const second = project({ name: "Page two" });
    server.use(
      getListProjectsMockHandler(({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        return cursor === "c2" ? page([second]) : page([first], "c2");
      }),
    );
    renderRoute("/");

    expect(await screen.findByText("Page one")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Page two")).toBeInTheDocument();
    expect(screen.getByText("Page one")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load more" })).not.toBeInTheDocument();
  });

  it("registers a project and posts the settings", async () => {
    const created = project({ name: "New project" });
    let body: unknown;
    renderRoute("/", {
      handlers: [
        getListProjectsMockHandler(page([])),
        getCreateProjectMockHandler(async ({ request }) => {
          body = await request.json();
          return created;
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Register project" }));
    const dialog = screen.getByRole("dialog", { name: "Register project" });
    expect(dialog).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("Name"), "New project");
    await userEvent.type(screen.getByLabelText("Description"), "Fresh");
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Register" }));

    await waitFor(() =>
      expect(body).toEqual({
        name: "New project",
        description: "Fresh",
        settings: { review_gate: false },
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Register project" })).not.toBeInTheDocument(),
    );
  });

  it("blocks submission on client-side validation errors", async () => {
    renderRoute("/", { handlers: [getListProjectsMockHandler(page([]))] });
    await userEvent.click(await screen.findByRole("button", { name: "Register project" }));
    await userEvent.click(screen.getByRole("button", { name: "Register" }));

    expect(screen.getByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
    // Dialog stays open, nothing was submitted.
    expect(screen.getByRole("dialog", { name: "Register project" })).toBeInTheDocument();
  });

  it("maps server 422 validation errors onto fields", async () => {
    renderRoute("/", {
      handlers: [
        getListProjectsMockHandler(page([])),
        http.post(PROJECTS_URL, () =>
          problemResponse(422, "validation-error", {
            title: "Validation failed",
            errors: [{ field: "/name", message: "name already taken" }],
          }),
        ),
      ],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Register project" }));
    await userEvent.type(screen.getByLabelText("Name"), "Duplicate");
    await userEvent.click(screen.getByRole("button", { name: "Register" }));

    expect(await screen.findByText("name already taken")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Register project" })).toBeInTheDocument();
  });

  it("imports an export document and reports counts", async () => {
    const projectId = uuid();
    let body: unknown;
    renderRoute("/", {
      handlers: [
        getListProjectsMockHandler(page([])),
        getImportProjectMockHandler(async ({ request }) => {
          body = await request.json();
          return {
            project_id: projectId,
            task_count: 3,
            relation_count: 2,
            session_count: 1,
            knowledge_count: 4,
          };
        }),
      ],
    });

    await userEvent.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = screen.getByRole("dialog", { name: "Import project" });
    const file = new File(
      [JSON.stringify({ version: "1.0.0", project: { name: "X" } })],
      "export.json",
      { type: "application/json" },
    );
    await userEvent.upload(screen.getByLabelText("Export file"), file);
    await userEvent.click(within(dialog).getByRole("button", { name: "Import" }));

    expect(
      await screen.findByText("Imported 3 tasks, 2 relations, 1 sessions, 4 knowledge items."),
    ).toBeInTheDocument();
    expect(body).toEqual({ version: "1.0.0", project: { name: "X" } });
  });

  it("rejects unparseable import files client-side", async () => {
    renderRoute("/", { handlers: [getListProjectsMockHandler(page([]))] });
    await userEvent.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = screen.getByRole("dialog", { name: "Import project" });
    const file = new File(["{not json"], "broken.json", {
      type: "application/json",
    });
    await userEvent.upload(screen.getByLabelText("Export file"), file);
    await userEvent.click(within(dialog).getByRole("button", { name: "Import" }));

    expect(await screen.findByText("That file is not valid JSON.")).toBeInTheDocument();
  });

  it("requires choosing a file before importing", async () => {
    renderRoute("/", { handlers: [getListProjectsMockHandler(page([]))] });
    await userEvent.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = screen.getByRole("dialog", { name: "Import project" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Import" }));
    expect(await screen.findByText("Choose an export file first.")).toBeInTheDocument();
  });
});
