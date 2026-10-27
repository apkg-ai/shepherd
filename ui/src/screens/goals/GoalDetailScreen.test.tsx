import { http } from "msw";
import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  goalHandler,
  epicListHandler,
  problemResponse,
} from "../../test/msw";
import { makeProject, makeGoal, makeEpic } from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game" });
const goal = makeGoal({
  id: "g1",
  project_id: "p1",
  title: "Combat system",
  epic_counts: { total: 2, done: 1, cancelled: 0, waived: 0 },
});

const baseHandlers = [sessionHandler(), projectHandler(project), goalHandler(goal)];

describe("GoalDetailScreen", () => {
  it("shows goal title, breadcrumbs, and epic count", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler([])],
    });

    expect(await screen.findByRole("heading", { name: "Combat system" })).toBeInTheDocument();
    expect(screen.getByText("2 epics")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Space Game" })).toHaveAttribute(
      "href",
      "/projects/p1",
    );
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
  });

  it("shows empty epic state with hint", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler([])],
    });

    expect(await screen.findByText("No epics yet")).toBeInTheDocument();
    expect(screen.getByText("Epics will appear here once created.")).toBeInTheDocument();
  });

  it("shows epic cards with status and task count", async () => {
    const epics = [
      makeEpic({
        id: "e1",
        project_id: "p1",
        goal_id: "g1",
        title: "Damage model",
        status: "active",
        task_counts: { total: 3, done: 1, cancelled: 0, waived: 0 },
      }),
      makeEpic({
        id: "e2",
        project_id: "p1",
        goal_id: "g1",
        title: "Targeting",
        status: "open",
        task_counts: { total: 1, done: 0, cancelled: 0, waived: 0 },
      }),
    ];
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics)],
    });

    expect(await screen.findByText("Damage model")).toBeInTheDocument();
    expect(screen.getByText("Targeting")).toBeInTheDocument();
    expect(screen.getByText("active")).toBeInTheDocument();
    expect(screen.getByText("3 tasks")).toBeInTheDocument();
  });

  it("has an edit link to the goal edit page", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler([])],
    });

    const editButton = await screen.findByRole("link", { name: /edit/i });
    expect(editButton).toHaveAttribute("href", "/projects/p1/goals/g1/edit");
  });

  it("shows error state when project fetch fails", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Project Error" }),
        ),
        goalHandler(goal),
        epicListHandler([]),
      ],
    });

    expect(await screen.findByText("Project Error")).toBeInTheDocument();
  });

  it("shows error state when epic list fails", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        goalHandler(goal),
        http.get("/api/v1/projects/:projectId/epics", () =>
          problemResponse(500, "internal-error", { title: "Epic Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Epic Error")).toBeInTheDocument();
  });

  it("shows error state when goal fetch fails", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [
        sessionHandler(),
        projectHandler(project),
        http.get("/api/v1/projects/:projectId/goals/g1", () =>
          problemResponse(500, "internal-error", { title: "Server Error" }),
        ),
        epicListHandler([]),
      ],
    });

    expect(await screen.findByText("Server Error")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
});
