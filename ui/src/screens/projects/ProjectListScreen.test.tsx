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
});
