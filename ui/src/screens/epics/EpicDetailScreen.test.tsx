import { http } from "msw";
import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  problemResponse,
  goalHandler,
  epicHandler,
  taskListHandler,
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

  it("shows description", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText("All combat rule logic")).toBeInTheDocument();
  });

  it("shows status badge", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText("Active")).toBeInTheDocument();
  });

  it("shows progress counts when tasks exist", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText(/1 of 3/)).toBeInTheDocument();
  });

  it("shows edit link", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Edit" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1/edit",
    );
  });

  it("shows new task link", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "New task" })).toHaveAttribute(
      "href",
      "/projects/p1/epics/e1/tasks/new",
    );
  });

  it("shows empty state when no tasks", async () => {
    renderRoute("/projects/p1/epics/e1", { handlers: baseHandlers });

    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
    expect(screen.getByText("Tasks will appear here once created.")).toBeInTheDocument();
  });

  it("shows task cards with status badges", async () => {
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
      ],
    });

    expect(await screen.findByText("Implement targeting")).toBeInTheDocument();
    expect(screen.getByText("Add damage model")).toBeInTheDocument();
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
      ],
    });

    expect(await screen.findByText("Eligibility")).toBeInTheDocument();
    expect(screen.getByText("Can plan")).toBeInTheDocument();
    expect(screen.getByText("Prerequisite epic not complete")).toBeInTheDocument();
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
});
