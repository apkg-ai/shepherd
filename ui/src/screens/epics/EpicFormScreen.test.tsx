import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  problemResponse,
  goalHandler,
  taskListHandler,
} from "../../test/msw";
import { server } from "../../test/msw";
import { makeProject, makeGoal, makeEpic, makePage } from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });
const goal = makeGoal({ id: "g1", project_id: "p1", title: "Combat" });

const baseHandlers = [sessionHandler(), projectHandler(project), goalHandler(goal)];

describe("EpicFormScreen — create mode", () => {
  it("renders create form with title and description fields", async () => {
    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    expect(await screen.findByRole("heading", { name: "New epic" })).toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toBeInTheDocument();
    expect(screen.getByLabelText("Description")).toBeInTheDocument();
  });

  it("submit is disabled when title is empty", async () => {
    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    const submit = await screen.findByRole("button", { name: "Create epic" });
    expect(submit).toBeDisabled();
  });

  it("submits and navigates to new epic on success", async () => {
    const user = userEvent.setup();
    const createdEpic = makeEpic({
      id: "new-e",
      project_id: "p1",
      goal_id: "g1",
      title: "New epic",
    });

    server.use(
      http.post("/api/v1/projects/p1/epics", () => HttpResponse.json(createdEpic, { status: 201 })),
      http.get("/api/v1/projects/:projectId/epics", () => HttpResponse.json(makePage([]))),
      http.get("/api/v1/projects/:projectId/epics/new-e", () => HttpResponse.json(createdEpic)),
      http.get("/api/v1/projects/:projectId/tasks", () => HttpResponse.json(makePage([]))),
    );

    const { router } = renderRoute("/projects/p1/goals/g1/epics/new", {
      handlers: baseHandlers,
    });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "New epic");
    await user.click(screen.getByRole("button", { name: "Create epic" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/epics/new-e"));
  });

  it("shows field errors on 422", async () => {
    const user = userEvent.setup();

    server.use(
      http.post("/api/v1/projects/p1/epics", () =>
        problemResponse(422, "validation_error", {
          title: "Validation failed",
          errors: [{ field: "/title", message: "Title is required", code: "required" }],
        }),
      ),
    );

    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Create epic" }));

    expect(await screen.findByText("Title is required")).toBeInTheDocument();
  });

  it("reuses the idempotency key after a non-422 create failure", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];
    server.use(
      http.post("/api/v1/projects/p1/epics", ({ request }) => {
        keys.push(request.headers.get("idempotency-key"));
        return problemResponse(500, "internal-error", { title: "Server Error" });
      }),
    );

    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "Test");
    await user.click(screen.getByRole("button", { name: "Create epic" }));
    expect(await screen.findByText("Server Error")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Create epic" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).toBe(keys[0]);
  });

  it("rotates the idempotency key on 422", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];
    server.use(
      http.post("/api/v1/projects/p1/epics", ({ request }) => {
        keys.push(request.headers.get("idempotency-key"));
        return problemResponse(422, "validation_error", {
          title: "Validation failed",
          errors: [{ field: "/title", message: "Too short", code: "min_length" }],
        });
      }),
    );

    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Create epic" }));
    expect(await screen.findByText("Too short")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Create epic" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).not.toBe(keys[0]);
  });

  it("explains an idempotency conflict and links to the goal", async () => {
    const user = userEvent.setup();
    server.use(
      http.post("/api/v1/projects/p1/epics", () =>
        problemResponse(409, "idempotency_conflict", { title: "Conflict" }),
      ),
    );

    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "Edited");
    await user.click(screen.getByRole("button", { name: "Create epic" }));

    expect(
      await screen.findByText(
        "A previous submission may already have created this epic. Check the goal epics before submitting again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Check goal epics" })).toHaveAttribute(
      "href",
      "/projects/p1/goals/g1",
    );
  });

  it("shows generic form error on non-422 create failure", async () => {
    const user = userEvent.setup();

    server.use(
      http.post("/api/v1/projects/p1/epics", () =>
        problemResponse(500, "internal-error", { title: "Server Error" }),
      ),
    );

    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "Test");
    await user.click(screen.getByRole("button", { name: "Create epic" }));

    expect(await screen.findByText("Server Error")).toBeInTheDocument();
  });

  it("shows error when project fails to load", async () => {
    renderRoute("/projects/p1/goals/g1/epics/new", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Project Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Project Error")).toBeInTheDocument();
  });

  it("shows breadcrumbs back to goal", async () => {
    renderRoute("/projects/p1/goals/g1/epics/new", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Combat" })).toHaveAttribute(
      "href",
      "/projects/p1/goals/g1",
    );
    expect(screen.getByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
  });

  it("cancel navigates back to goal when form is clean", async () => {
    const { router } = renderRoute("/projects/p1/goals/g1/epics/new", {
      handlers: baseHandlers,
    });

    await screen.findByRole("heading", { name: "New epic" });
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/goals/g1"));
  });
});

describe("EpicFormScreen — edit mode", () => {
  const existingEpic = makeEpic({
    id: "e1",
    project_id: "p1",
    goal_id: "g1",
    title: "Original title",
    description: "Original desc",
    revision: 1,
  });

  const editHandlers = [
    ...baseHandlers,
    http.get("/api/v1/projects/:projectId/epics/e1", () => HttpResponse.json(existingEpic)),
    taskListHandler([]),
  ];

  it("loads existing epic data into form", async () => {
    renderRoute("/projects/p1/epics/e1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    expect(screen.getByLabelText("Description")).toHaveValue("Original desc");
  });

  it("shows conflict comparison on 412", async () => {
    const user = userEvent.setup();
    const updated = makeEpic({
      ...existingEpic,
      title: "Updated by someone",
      revision: 2,
    });

    let epicGetCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/epics/e1", () => {
        epicGetCount++;
        if (epicGetCount <= 1) return HttpResponse.json(existingEpic);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/epics/e1", () =>
        problemResponse(412, "revision_conflict", { title: "Stale revision" }),
      ),
    ];

    renderRoute("/projects/p1/epics/e1/edit", { handlers: conflictHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "My edit");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    expect(screen.getByText("Your version")).toBeInTheDocument();
    expect(screen.getByText("Current version")).toBeInTheDocument();
    expect(screen.getByText("My edit")).toBeInTheDocument();
    expect(screen.getAllByText("Updated by someone").length).toBeGreaterThanOrEqual(1);
  });

  it("cancel navigates back without changes if form is clean", async () => {
    const { router } = renderRoute("/projects/p1/epics/e1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/epics/e1"));
  });

  it("shows discard confirmation when form is dirty", async () => {
    const user = userEvent.setup();
    renderRoute("/projects/p1/epics/e1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " changed");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(await screen.findByText("Discard changes?")).toBeInTheDocument();
  });

  it("submits edit and navigates to epic detail on success", async () => {
    const user = userEvent.setup();
    const updatedEpic = makeEpic({ ...existingEpic, title: "Updated", revision: 2 });

    renderRoute("/projects/p1/epics/e1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/epics/e1", () => HttpResponse.json(existingEpic)),
        http.patch("/api/v1/projects/p1/epics/e1", () => HttpResponse.json(updatedEpic)),
        taskListHandler([]),
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

  it("conflict 'Save anyway' resubmits with the remote revision", async () => {
    const user = userEvent.setup();
    const updated = makeEpic({
      ...existingEpic,
      title: "Updated by someone",
      revision: 2,
    });

    let epicGetCount = 0;
    let patchCount = 0;
    const ifMatchHeaders: Array<string | null> = [];
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/epics/e1", () => {
        epicGetCount++;
        if (epicGetCount <= 1) return HttpResponse.json(existingEpic);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/epics/e1", ({ request }) => {
        patchCount++;
        ifMatchHeaders.push(request.headers.get("if-match"));
        if (patchCount <= 1) return problemResponse(412, "revision_conflict", { title: "Stale" });
        return HttpResponse.json({ ...updated, title: "My edit" });
      }),
      taskListHandler([]),
    ];

    const { router } = renderRoute("/projects/p1/epics/e1/edit", {
      handlers: conflictHandlers,
    });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "My edit");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save anyway" }));

    await waitFor(() => expect(screen.queryByText("Edit conflict")).not.toBeInTheDocument());
    expect(screen.getByLabelText("Title")).toHaveValue("My edit");

    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/epics/e1"));
    expect(ifMatchHeaders).toEqual([`"1"`, `"2"`]);
    expect(patchCount).toBe(2);
    expect(epicGetCount).toBeGreaterThanOrEqual(2);
  });

  it("conflict 'Discard my changes' navigates back to epic detail", async () => {
    const user = userEvent.setup();
    const updated = makeEpic({ ...existingEpic, title: "Updated", revision: 2 });

    let epicGetCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/epics/e1", () => {
        epicGetCount++;
        if (epicGetCount <= 1) return HttpResponse.json(existingEpic);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/epics/e1", () =>
        problemResponse(412, "revision_conflict", { title: "Stale" }),
      ),
      taskListHandler([]),
    ];

    const { router } = renderRoute("/projects/p1/epics/e1/edit", {
      handlers: conflictHandlers,
    });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original title"));
    await user.type(screen.getByLabelText("Title"), " changed");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Discard my changes" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/epics/e1"));
  });

  it("shows field errors on edit 422", async () => {
    const user = userEvent.setup();

    renderRoute("/projects/p1/epics/e1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/epics/e1", () => HttpResponse.json(existingEpic)),
        http.patch("/api/v1/projects/p1/epics/e1", () =>
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

    renderRoute("/projects/p1/epics/e1/edit", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/epics/e1", () => HttpResponse.json(existingEpic)),
        http.patch("/api/v1/projects/p1/epics/e1", () =>
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
