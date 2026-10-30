import { http, HttpResponse } from "msw";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { makeProject, makePage } from "../../test/fixtures";
import { sessionHandler, projectListHandler, problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

const project1 = makeProject({ name: "Space Game", description: "A galactic adventure" });
const project2 = makeProject({ name: "Website Redesign" });

describe("ProjectListScreen", () => {
  it("shows project cards when projects exist", async () => {
    renderRoute("/", {
      handlers: [sessionHandler(), projectListHandler([project1, project2])],
    });

    expect(await screen.findByText("Space Game")).toBeInTheDocument();
    expect(screen.getByText("Website Redesign")).toBeInTheDocument();
    expect(screen.getByText("A galactic adventure")).toBeInTheDocument();
  });

  it("shows empty state with CTA when no projects exist", async () => {
    renderRoute("/", {
      handlers: [sessionHandler(), projectListHandler([])],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create your first project" })).toBeInTheDocument();
  });

  it("project cards link to /projects/:id", async () => {
    renderRoute("/", {
      handlers: [sessionHandler(), projectListHandler([project1])],
    });

    const link = await screen.findByRole("link", { name: /Space Game/ });
    expect(link).toHaveAttribute("href", `/projects/${project1.id}`);
  });

  it("opens the create project dialog and submits", async () => {
    const user = userEvent.setup();
    const created = makeProject({ name: "New Project" });

    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", () => HttpResponse.json(created, { status: 201 })),
        http.get(`/api/v1/projects/${created.id}`, () => HttpResponse.json(created)),
        http.get("/api/v1/projects/:projectId/goals", () => HttpResponse.json(makePage([]))),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Name"), "New Project");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("shows validation errors in the create dialog on 422", async () => {
    const user = userEvent.setup();
    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", () =>
          problemResponse(422, "validation_error", {
            title: "Validation failed",
            errors: [{ field: "/name", message: "Name is required", code: "required" }],
          }),
        ),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "x");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));

    expect(await within(dialog).findByText("Name is required")).toBeInTheDocument();
  });

  it("closes the create dialog and resets form", async () => {
    const user = userEvent.setup();
    renderRoute("/", { handlers: [sessionHandler(), projectListHandler([])] });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Draft");
    await user.type(within(dialog).getByLabelText("Description"), "Some description");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows generic error in create dialog on non-422 failure", async () => {
    const user = userEvent.setup();
    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", () =>
          problemResponse(500, "internal-error", { title: "Server Error" }),
        ),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Test");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));

    expect(await within(dialog).findByText("Server Error")).toBeInTheDocument();
  });

  it("reuses the idempotency key after a non-422 create failure", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];
    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", ({ request }) => {
          keys.push(request.headers.get("idempotency-key"));
          return problemResponse(500, "internal-error", { title: "Server Error" });
        }),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Test");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));
    expect(await within(dialog).findByText("Server Error")).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Create project" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).toBe(keys[0]);
  });

  it("rotates the idempotency key on 422", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];
    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", ({ request }) => {
          keys.push(request.headers.get("idempotency-key"));
          return problemResponse(422, "validation_error", {
            title: "Validation failed",
            errors: [{ field: "/name", message: "Name is required", code: "required" }],
          });
        }),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "x");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));
    expect(await within(dialog).findByText("Name is required")).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Create project" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).not.toBe(keys[0]);
  });

  it("explains an idempotency conflict without rotating the key", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];
    renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.post("/api/v1/projects", ({ request }) => {
          keys.push(request.headers.get("idempotency-key"));
          return problemResponse(409, "idempotency_conflict", {
            title: "Conflict",
            detail: "This idempotency key was already used with a different request.",
          });
        }),
      ],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));

    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Test");
    await user.click(within(dialog).getByRole("button", { name: "Create project" }));

    expect(
      await within(dialog).findByText(
        "This idempotency key was already used with a different request.",
      ),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Create project" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).toBe(keys[0]);
  });
});
