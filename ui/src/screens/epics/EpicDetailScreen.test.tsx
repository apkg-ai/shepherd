import { http } from "msw";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  problemResponse,
  goalHandler,
  epicHandler,
  taskListHandler,
  epicGraphHandler,
} from "../../test/msw";
import { makeProject, makeGoal, makeEpic, makeTask } from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });
const goal = makeGoal({ id: "g1", project_id: "p1", title: "Combat" });
const epic = makeEpic({
  id: "e1",
  project_id: "p1",
  goal_id: "g1",
  title: "Combat rules",
  description: "All combat rule logic",
  status: "active",
  task_counts: { total: 3, done: 1, cancelled: 0, waived: 0 },
});

const baseHandlers = [
  sessionHandler(),
  projectHandler(project),
  goalHandler(goal),
  epicHandler(epic),
  taskListHandler([]),
  epicGraphHandler(),
];

describe("EpicDetailScreen", () => {
  it("shows epic title and breadcrumbs", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByRole("heading", { name: "Combat rules" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
    expect(screen.getByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
    expect(screen.getByRole("link", { name: "Combat" })).toHaveAttribute(
      "href",
      "/projects/p1/goals/g1",
    );
  });

  it("shows status badge", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText("Active")).toBeInTheDocument();
  });

  it("shows progress counts when tasks exist", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText(/1 of 3/)).toBeInTheDocument();
  });

  it("shows edit and new task links in the page header", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Edit" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1/edit",
    );

    expect(screen.getByRole("link", { name: "New task" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1/tasks/new",
    );
  });

  it("shows empty state when no tasks", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
    expect(screen.getByText("Tasks will appear here once created.")).toBeInTheDocument();
  });

  it("shows eligibility panel", async () => {
    const epicWithReasons = makeEpic({
      ...epic,
      eligibility: {
        can_plan: true,
        can_execute: false,
        can_review: false,
        reasons: [
          {
            code: "epic_prerequisite",
            resource_id: "e2",
            message: "Prerequisite epic not complete",
          },
        ],
        allowed_actions: [],
      },
    });

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epicWithReasons),
        taskListHandler([]),
        epicGraphHandler(),
      ],
    });

    expect(await screen.findByText("Eligibility")).toBeInTheDocument();
    expect(screen.getByText("Can plan: Yes")).toBeInTheDocument();
    expect(screen.getByText("Prerequisite epic not complete")).toBeInTheDocument();
  });

  it("shows task cards with status badges in list view", async () => {
    const user = userEvent.setup();
    const tasks = [
      makeTask({
        id: "t1",
        project_id: "p1",
        epic_id: "e1",
        title: "Implement targeting",
        status: "active",
      }),
      makeTask({
        id: "t2",
        project_id: "p1",
        epic_id: "e1",
        title: "Add damage model",
        status: "open",
      }),
    ];

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        taskListHandler(tasks),
        epicGraphHandler(),
      ],
    });

    await user.click(await screen.findByRole("button", { name: /List view/ }));

    expect(await screen.findByText("Implement targeting")).toBeInTheDocument();
    expect(screen.getByText("Add damage model")).toBeInTheDocument();
  });

  it("shows error when epic fails to load", async () => {
    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        http.get("/api/v1/projects/:projectId/epics/e1", () =>
          problemResponse(500, "internal-error", { title: "Epic Error" }),
        ),
        taskListHandler([]),
      ],
    });

    expect(await screen.findByText("Epic Error")).toBeInTheDocument();
  });

  it("shows error when project fails to load", async () => {
    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Project Error" }),
        ),
        taskListHandler([]),
      ],
    });

    expect(await screen.findByText("Project Error")).toBeInTheDocument();
  });

  it("default filter hides done/cancelled tasks in list view", async () => {
    const user = userEvent.setup();
    const tasks = [
      makeTask({ id: "t1", project_id: "p1", epic_id: "e1", title: "Active task", status: "open" }),
      makeTask({
        id: "t2",
        project_id: "p1",
        epic_id: "e1",
        title: "Cancelled task",
        status: "cancelled",
      }),
    ];

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        taskListHandler(tasks),
        epicGraphHandler(),
      ],
    });

    await user.click(await screen.findByRole("button", { name: /List view/ }));

    expect(await screen.findByText("Active task")).toBeInTheDocument();
    expect(screen.queryByText("Cancelled task")).not.toBeInTheDocument();
  });

  it("Done filter shows only done/cancelled tasks", async () => {
    const user = userEvent.setup();
    const tasks = [
      makeTask({ id: "t1", project_id: "p1", epic_id: "e1", title: "Active task", status: "open" }),
      makeTask({
        id: "t2",
        project_id: "p1",
        epic_id: "e1",
        title: "Done task",
        status: "done",
      }),
    ];

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        taskListHandler(tasks),
        epicGraphHandler(),
      ],
    });

    await user.click(await screen.findByRole("button", { name: /List view/ }));
    await screen.findByText("Active task");
    await user.click(screen.getByRole("button", { name: /Done/ }));

    expect(await screen.findByText("Done task")).toBeInTheDocument();
    expect(screen.queryByText("Active task")).not.toBeInTheDocument();
  });

  it("All filter shows everything", async () => {
    const user = userEvent.setup();
    const tasks = [
      makeTask({ id: "t1", project_id: "p1", epic_id: "e1", title: "Active task", status: "open" }),
      makeTask({
        id: "t2",
        project_id: "p1",
        epic_id: "e1",
        title: "Done task",
        status: "done",
      }),
    ];

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        taskListHandler(tasks),
        epicGraphHandler(),
      ],
    });

    await user.click(await screen.findByRole("button", { name: /List view/ }));
    await screen.findByText("Active task");
    await user.click(screen.getByRole("button", { name: /All/ }));

    expect(await screen.findByText("Active task")).toBeInTheDocument();
    expect(screen.getByText("Done task")).toBeInTheDocument();
  });

  it("shows no-results message when filter matches nothing", async () => {
    const user = userEvent.setup();
    const tasks = [
      makeTask({ id: "t1", project_id: "p1", epic_id: "e1", title: "Active task", status: "open" }),
    ];

    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        taskListHandler(tasks),
        epicGraphHandler(),
      ],
    });

    await user.click(await screen.findByRole("button", { name: /List view/ }));
    await screen.findByText("Active task");
    await user.click(screen.getByRole("button", { name: /Done/ }));

    expect(await screen.findByText("No items match this filter.")).toBeInTheDocument();
  });

  it("shows error when goal fails to load", async () => {
    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        epicHandler(epic),
        http.get("/api/v1/projects/:projectId/goals/g1", () =>
          problemResponse(500, "internal-error", { title: "Goal Error" }),
        ),
        taskListHandler([]),
      ],
    });

    expect(await screen.findByText("Goal Error")).toBeInTheDocument();
  });

  it("shows error when tasks fail to load", async () => {
    renderRoute("/projects/p1/epics/e1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        epicHandler(epic),
        http.get("/api/v1/projects/:projectId/tasks", () =>
          problemResponse(500, "internal-error", { title: "Tasks Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Tasks Error")).toBeInTheDocument();
  });
});
