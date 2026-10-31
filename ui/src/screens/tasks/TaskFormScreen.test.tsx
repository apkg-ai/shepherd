import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  problemResponse,
  epicHandler,
  taskTypeListHandler,
  server,
} from "../../test/msw";
import {
  makeProject,
  makeGoal,
  makeEpic,
  makeTask,
  makeTaskType,
  makePage,
} from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });
const goal = makeGoal({ id: "g1", project_id: "p1", title: "Combat" });
const epic = makeEpic({
  id: "e1",
  project_id: "p1",
  goal_id: "g1",
  title: "Combat Rules",
});
const taskTypes = [
  makeTaskType({ key: "code", label: "Code", project_id: "p1" }),
  makeTaskType({ key: "research", label: "Research", project_id: "p1" }),
];

const baseHandlers = [
  sessionHandler(),
  projectHandler(project),
  http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(goal)),
  epicHandler(epic),
  taskTypeListHandler(taskTypes),
  http.get("/api/v1/projects/:projectId/tasks", () => HttpResponse.json(makePage([]))),
];

describe("TaskFormScreen — create mode", () => {
  it("renders create form with all fields", async () => {
    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    expect(await screen.findByRole("heading", { name: "New task" })).toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toBeInTheDocument();
    expect(screen.getByLabelText("Description")).toBeInTheDocument();
    expect(screen.getByLabelText("Type")).toBeInTheDocument();
    expect(screen.getByLabelText("Planning required")).toBeInTheDocument();
    expect(screen.getByLabelText("Plan review")).toBeInTheDocument();
    expect(screen.getByLabelText("Work review")).toBeInTheDocument();
  });

  it("submit is disabled when title is empty", async () => {
    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    const submit = await screen.findByRole("button", { name: "Create task" });
    expect(submit).toBeDisabled();
  });

  it("type dropdown shows loaded task types only", async () => {
    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    await screen.findByLabelText("Type");
    const options = screen.getByLabelText("Type").querySelectorAll("option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("Code");
    expect(options[1]).toHaveTextContent("Research");
  });

  it("submits and navigates to new task on success", async () => {
    const user = userEvent.setup();
    const createdTask = makeTask({
      id: "new-t",
      project_id: "p1",
      epic_id: "e1",
      title: "Targeting",
    });

    server.use(
      http.post("/api/v1/projects/p1/tasks", () => HttpResponse.json(createdTask, { status: 201 })),
      http.get("/api/v1/projects/:projectId/tasks/new-t", () => HttpResponse.json(createdTask)),
      http.get("/api/v1/projects/:projectId/epics/:epicId", () => HttpResponse.json(epic)),
      http.get("/api/v1/projects/:projectId/goals/:goalId", () => HttpResponse.json(goal)),
    );

    const { router } = renderRoute("/projects/p1/epics/e1/tasks/new", {
      handlers: baseHandlers,
    });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "Targeting");
    await user.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/tasks/new-t"));
  });

  it("shows field errors on 422", async () => {
    const user = userEvent.setup();

    server.use(
      http.post("/api/v1/projects/p1/tasks", () =>
        problemResponse(422, "validation_error", {
          title: "Validation failed",
          errors: [{ field: "/title", message: "Title is required", code: "required" }],
        }),
      ),
    );

    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Create task" }));

    expect(await screen.findByText("Title is required")).toBeInTheDocument();
  });

  it("rotates the idempotency key on 422", async () => {
    const user = userEvent.setup();
    const keys: Array<string | null> = [];

    server.use(
      http.post("/api/v1/projects/p1/tasks", ({ request }) => {
        keys.push(request.headers.get("idempotency-key"));
        return problemResponse(422, "validation_error", {
          title: "Validation failed",
          errors: [{ field: "/title", message: "Too short", code: "min_length" }],
        });
      }),
    );

    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    await screen.findByLabelText("Title");
    await user.type(screen.getByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Create task" }));
    expect(await screen.findByText("Too short")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).not.toBe(keys[0]);
  });

  it("shows breadcrumbs back to epic", async () => {
    renderRoute("/projects/p1/epics/e1/tasks/new", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Combat Rules" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1",
    );
    expect(screen.getByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
  });

  it("cancel navigates back to epic when form is clean", async () => {
    const { router } = renderRoute("/projects/p1/epics/e1/tasks/new", {
      handlers: baseHandlers,
    });

    await screen.findByRole("heading", { name: "New task" });
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1/epics/e1"));
  });
});

describe("TaskFormScreen — edit mode", () => {
  const existingTask = makeTask({
    id: "t1",
    project_id: "p1",
    epic_id: "e1",
    title: "Original task",
    description: "Original desc",
    type_key: "code",
    revision: 1,
  });

  const editHandlers = [
    ...baseHandlers,
    http.get("/api/v1/projects/:projectId/tasks/t1", () => HttpResponse.json(existingTask)),
  ];

  it("loads existing task data into form", async () => {
    renderRoute("/projects/p1/tasks/t1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original task"));
    expect(screen.getByLabelText("Description")).toHaveValue("Original desc");
  });

  it("shows conflict comparison on 412", async () => {
    const user = userEvent.setup();
    const updated = makeTask({
      ...existingTask,
      title: "Updated by someone",
      revision: 2,
    });

    let taskGetCount = 0;
    const conflictHandlers = [
      ...baseHandlers,
      http.get("/api/v1/projects/:projectId/tasks/t1", () => {
        taskGetCount++;
        if (taskGetCount <= 1) return HttpResponse.json(existingTask);
        return HttpResponse.json(updated);
      }),
      http.patch("/api/v1/projects/p1/tasks/t1", () =>
        problemResponse(412, "revision_conflict", { title: "Stale revision" }),
      ),
    ];

    renderRoute("/projects/p1/tasks/t1/edit", { handlers: conflictHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original task"));
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "My edit");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Edit conflict")).toBeInTheDocument();
    expect(screen.getByText("Your version")).toBeInTheDocument();
    expect(screen.getByText("Current version")).toBeInTheDocument();
    expect(screen.getByText("My edit")).toBeInTheDocument();
    expect(screen.getAllByText("Updated by someone").length).toBeGreaterThanOrEqual(1);
  });

  it("shows discard confirmation when form is dirty", async () => {
    const user = userEvent.setup();
    renderRoute("/projects/p1/tasks/t1/edit", { handlers: editHandlers });

    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Original task"));
    await user.type(screen.getByLabelText("Title"), " changed");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(await screen.findByText("Discard changes?")).toBeInTheDocument();
  });

  it("shows error when project fails to load", async () => {
    renderRoute("/projects/p1/tasks/t1/edit", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Project Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Project Error")).toBeInTheDocument();
  });
});
