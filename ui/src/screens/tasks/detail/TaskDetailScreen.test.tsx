import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { sessionHandler, projectHandler, problemResponse } from "../../../test/msw";
import { makeProject, makeGoal, makeEpic, makeTask } from "../../../test/fixtures";
import { renderRoute } from "../../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });
const goal = makeGoal({ id: "g1", project_id: "p1", title: "Combat" });
const epic = makeEpic({
  id: "e1",
  project_id: "p1",
  goal_id: "g1",
  title: "Combat Rules",
});
const task = makeTask({
  id: "t1",
  project_id: "p1",
  epic_id: "e1",
  title: "Implement targeting",
  description: "Target selection logic",
  type_key: "code",
  status: "active",
  phase: "execution",
  planning_required: true,
  plan_review: "agent",
  work_review: "human",
  eligibility: {
    can_plan: false,
    can_execute: true,
    can_review: false,
    reasons: [
      {
        code: "task_prerequisite",
        resource_id: "t2",
        message: "Blocked by damage model",
      },
    ],
    allowed_actions: ["claim_execute"],
  },
});

const baseHandlers = [
  sessionHandler(),
  projectHandler(project),
  http.get("/api/v1/projects/:projectId/goals/g1", () => HttpResponse.json(goal)),
  http.get("/api/v1/projects/:projectId/epics/e1", () => HttpResponse.json(epic)),
  http.get("/api/v1/projects/:projectId/tasks/t1", () => HttpResponse.json(task)),
];

describe("TaskDetailScreen", () => {
  it("renders task title and description", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    expect(await screen.findByRole("heading", { name: "Implement targeting" })).toBeInTheDocument();
    expect(screen.getByText("Target selection logic")).toBeInTheDocument();
  });

  it("shows status and phase badges", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    expect(await screen.findByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Execution")).toBeInTheDocument();
  });

  it("shows task metadata", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    const metadata = await screen.findByRole("region", { name: "Task metadata" });
    expect(metadata).toHaveTextContent("code");
    expect(metadata).toHaveTextContent("Yes");
    expect(metadata).toHaveTextContent("agent");
    expect(metadata).toHaveTextContent("human");
  });

  it("shows eligibility panel with gate reasons", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    expect(await screen.findByText("Blocked by damage model")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Blocked by damage model" })).toHaveAttribute(
      "href",
      "/projects/p1/tasks/t2",
    );
  });

  it("shows edit link", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    const editLink = await screen.findByRole("link", { name: "Edit" });
    expect(editLink).toHaveAttribute("href", "/projects/p1/tasks/t1/edit");
  });

  it("shows breadcrumbs to project", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    expect(await screen.findByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
  });

  it("renders task action buttons from allowed_actions", async () => {
    renderRoute("/projects/p1/tasks/t1", { handlers: baseHandlers });

    expect(await screen.findByRole("button", { name: "Claim execution" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Accept" })).not.toBeInTheDocument();
  });

  it("shows error when task fails to load", async () => {
    renderRoute("/projects/p1/tasks/t1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        http.get("/api/v1/projects/:projectId/tasks/t1", () =>
          problemResponse(500, "internal-error", { title: "Task Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Task Error")).toBeInTheDocument();
  });

  it("shows no actions when allowed_actions is empty", async () => {
    const taskNoActions = makeTask({
      ...task,
      eligibility: {
        ...task.eligibility,
        allowed_actions: [],
      },
    });

    renderRoute("/projects/p1/tasks/t1", {
      handlers: [
        ...baseHandlers.slice(0, -1),
        http.get("/api/v1/projects/:projectId/tasks/t1", () => HttpResponse.json(taskNoActions)),
      ],
    });

    await waitFor(() => expect(screen.getByText("No actions available")).toBeInTheDocument());
  });
});
