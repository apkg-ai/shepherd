import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { sessionHandler, projectHandler, problemResponse } from "../../test/msw";
import { server } from "../../test/msw";
import { makeProject, makeGoal, makePage } from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });

const baseHandlers = [sessionHandler(), projectHandler(project)];

describe("GoalFormScreen — create mode", () => {
  it("renders create form with title and description fields", async () => {
    renderRoute("/projects/p1/goals/new", { handlers: baseHandlers });

    expect(await screen.findByRole("heading", { name: "New goal" })).toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toBeInTheDocument();
    expect(screen.getByLabelText("Description")).toBeInTheDocument();
  });

  it("submit is disabled when title is empty", async () => {
    renderRoute("/projects/p1/goals/new", { handlers: baseHandlers });

    const submit = await screen.findByRole("button", { name: "Create goal" });
    expect(submit).toBeDisabled();
  });

  it("submits and navigates to new goal on success", async () => {
    const user = userEvent.setup();
    const createdGoal = makeGoal({ id: "new-g", project_id: "p1", title: "New goal" });

    server.use(
      http.post("/api/v1/projects/p1/goals", () => HttpResponse.json(createdGoal, { status: 201 })),
      http.get("/api/v1/projects/p1/goals", () => HttpResponse.json(makePage([]))),
      http.get("/api/v1/projects/:projectId/goals/new-g", () => HttpResponse.json(createdGoal)),
      http.get("/api/v1/projects/:projectId/epics", () => HttpResponse.json(makePage([]))),
    );

    const { router } = renderRoute("/projects/p1/goals/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "New goal");
    await user.click(screen.getByRole("button", { name: "Create goal" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/goals/new-g"));
  });

  it("shows field errors on 422", async () => {
    const user = userEvent.setup();

    server.use(
      http.post("/api/v1/projects/p1/goals", () =>
        problemResponse(422, "validation_error", {
          title: "Validation failed",
          errors: [{ field: "/title", message: "Title is required", code: "required" }],
        }),
      ),
    );

    renderRoute("/projects/p1/goals/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Create goal" }));

    expect(await screen.findByText("Title is required")).toBeInTheDocument();
  });

  it("shows error when project fails to load", async () => {
    renderRoute("/projects/p1/goals/new", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Project Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Project Error")).toBeInTheDocument();
  });

  it("shows error when goal fails to load in edit mode", async () => {
    renderRoute("/projects/p1/goals/g1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/goals/g1", () =>
          problemResponse(500, "internal-error", { title: "Goal Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Goal Error")).toBeInTheDocument();
  });

  it("shows breadcrumbs back to project", async () => {
    renderRoute("/projects/p1/goals/new", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
  });
});

describe("GoalFormScreen — edit mode", () => {
  const existingGoal = makeGoal({
    id: "g1",
    project_id: "p1",
    title: "Original title",
    description: "Original desc",
    revision: 1,
  });

  const editHandlers = [
    ...baseHandlers,
    http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(existingGoal)),
  ];

  it("loads existing goal data into form", async () => {
    renderRoute("/projects/p1/goals/g1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    expect(screen.getByLabelText("Description")).toHaveValue("Original desc");
  });

  it("shows conflict comparison on 412", async () => {
    const user = userEvent.setup();
    const updated = makeGoal({
      ...existingGoal,
      title: "Updated by someone",
      revision: 2,
    });

    let goalGetCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/goals/g1", () => {
        goalGetCount++;
        // First call loads the form; subsequent calls return the updated version
        if (goalGetCount <= 1) return HttpResponse.json(existingGoal);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/goals/g1", () =>
        problemResponse(412, "revision_conflict", { title: "Stale revision" }),
      ),
    ];

    renderRoute("/projects/p1/goals/g1/edit", { handlers: conflictHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "My edit");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    expect(screen.getByText("Your version")).toBeInTheDocument();
    expect(screen.getByText("Current version")).toBeInTheDocument();
    expect(screen.getByText("My edit")).toBeInTheDocument();
    // "Updated by someone" appears in both breadcrumb and conflict pane
    expect(screen.getAllByText("Updated by someone").length).toBeGreaterThanOrEqual(1);
  });

  it("cancel navigates back without changes if form is clean", async () => {
    const { router } = renderRoute("/projects/p1/goals/g1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/goals/g1"));
  });

  it("shows discard confirmation when form is dirty", async () => {
    const user = userEvent.setup();
    renderRoute("/projects/p1/goals/g1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " changed");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(await screen.findByText("Discard changes?")).toBeInTheDocument();
  });

  it("submits edit and navigates to goal detail on success", async () => {
    const user = userEvent.setup();
    const updatedGoal = makeGoal({ ...existingGoal, title: "Updated", revision: 2 });

    renderRoute("/projects/p1/goals/g1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(existingGoal)),
        http.patch("/api/v1/projects/p1/goals/g1", () => HttpResponse.json(updatedGoal)),
        http.get("/api/v1/projects/:projectId/epics", () => HttpResponse.json(makePage([]))),
      ],
    });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "Updated");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument(),
    );
  });

  it("conflict 'Save anyway' returns to the form with updated revision", async () => {
    const user = userEvent.setup();
    const updated = makeGoal({ ...existingGoal, title: "Updated by someone", revision: 2 });

    let goalGetCount = 0;
    let patchCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/goals/g1", () => {
        goalGetCount++;
        if (goalGetCount <= 1) return HttpResponse.json(existingGoal);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/goals/g1", () => {
        patchCount++;
        if (patchCount <= 1) return problemResponse(412, "revision_conflict", { title: "Stale" });
        return HttpResponse.json({ ...updated, title: "My edit" });
      }),
      http.get("/api/v1/projects/:projectId/epics", () => HttpResponse.json(makePage([]))),
    ];

    renderRoute("/projects/p1/goals/g1/edit", { handlers: conflictHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "My edit");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save anyway" }));

    // Returns to form — conflict is cleared
    await waitFor(() => expect(screen.queryByText("Edit conflict")).not.toBeInTheDocument());
    expect(screen.getByLabelText("Title")).toHaveValue("My edit");
  });

  it("conflict 'Discard my changes' navigates back to goal detail", async () => {
    const user = userEvent.setup();
    const updated = makeGoal({ ...existingGoal, title: "Updated", revision: 2 });

    let goalGetCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/goals/g1", () => {
        goalGetCount++;
        if (goalGetCount <= 1) return HttpResponse.json(existingGoal);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/goals/g1", () =>
        problemResponse(412, "revision_conflict", { title: "Stale" }),
      ),
      http.get("/api/v1/projects/:projectId/epics", () => HttpResponse.json(makePage([]))),
    ];

    const { router } = renderRoute("/projects/p1/goals/g1/edit", { handlers: conflictHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " changed");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Discard my changes" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/goals/g1"));
  });

  it("shows field errors on edit 422", async () => {
    const user = userEvent.setup();

    renderRoute("/projects/p1/goals/g1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(existingGoal)),
        http.patch("/api/v1/projects/p1/goals/g1", () =>
          problemResponse(422, "validation_error", {
            title: "Validation failed",
            errors: [{ field: "/title", message: "Too short", code: "min_length" }],
          }),
        ),
      ],
    });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " x");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Too short")).toBeInTheDocument();
  });

  it("shows form error on unexpected mutation failure", async () => {
    const user = userEvent.setup();

    renderRoute("/projects/p1/goals/g1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(existingGoal)),
        http.patch("/api/v1/projects/p1/goals/g1", () =>
          problemResponse(500, "internal-error", { title: "Server exploded" }),
        ),
      ],
    });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " x");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Server exploded")).toBeInTheDocument();
  });
});

describe("GoalFormScreen — create cancel", () => {
  it("cancel navigates back to project overview when form is clean", async () => {
    const project = makeProject({ id: "p1", name: "Space Game" });
    const { router } = renderRoute("/projects/p1/goals/new", {
      handlers: [sessionHandler(), projectHandler(project)],
    });

    await screen.findByRole("heading", { name: "New goal" });
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1"));
  });
});
