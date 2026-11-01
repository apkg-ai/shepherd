import { http } from "msw";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { makeProject, makeGoal } from "../../test/fixtures";
import {
  sessionHandler,
  projectHandler,
  goalListHandler,
  goalGraphHandler,
  problemResponse,
} from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Space Game", description: "A galactic adventure" });

const baseHandlers = [sessionHandler(), projectHandler(project)];

describe("ProjectOverviewScreen", () => {
  it("shows project title, description, and breadcrumbs", async () => {
    renderRoute("/projects/p1", {
      handlers: [...baseHandlers, goalListHandler([])],
    });

    expect(await screen.findByRole("heading", { name: "Space Game" })).toBeInTheDocument();
    expect(screen.getByText("A galactic adventure")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
  });

  it("shows goals in table view", async () => {
    const user = userEvent.setup();
    const goals = [
      makeGoal({
        id: "g1",
        project_id: "p1",
        title: "Combat system",
        epic_counts: { total: 3, done: 1, cancelled: 0, waived: 0 },
      }),
      makeGoal({
        id: "g2",
        project_id: "p1",
        title: "Economy",
        epic_counts: { total: 1, done: 0, cancelled: 0, waived: 0 },
      }),
    ];
    renderRoute("/projects/p1", {
      handlers: [...baseHandlers, goalListHandler(goals), goalGraphHandler()],
    });

    await screen.findByRole("heading", { name: "Space Game" });
    await user.click(screen.getByRole("button", { name: /Table view/ }));

    expect(await screen.findByText("Combat system")).toBeInTheDocument();
    expect(screen.getByText("Economy")).toBeInTheDocument();
  });

  it("shows empty state with CTA when no goals", async () => {
    renderRoute("/projects/p1", {
      handlers: [...baseHandlers, goalListHandler([])],
    });

    expect(await screen.findByText("No goals yet")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Create your first goal/ })).toBeInTheDocument();
  });

  it("goal links to goal detail in table view", async () => {
    const user = userEvent.setup();
    const goal = makeGoal({ id: "g1", project_id: "p1", title: "Combat" });
    renderRoute("/projects/p1", {
      handlers: [...baseHandlers, goalListHandler([goal]), goalGraphHandler()],
    });

    await screen.findByRole("heading", { name: "Space Game" });
    await user.click(screen.getByRole("button", { name: /Table view/ }));

    const link = await screen.findByRole("link", { name: /Combat/ });
    expect(link).toHaveAttribute("href", "/projects/p1/goals/g1");
  });

  it("shows error state when goal list fails", async () => {
    renderRoute("/projects/p1", {
      handlers: [
        ...baseHandlers,
        http.get("/api/v1/projects/:projectId/goals", () =>
          problemResponse(500, "internal-error", { title: "Server Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Server Error")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("shows error state when project fetch fails", async () => {
    renderRoute("/projects/p1", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(404, "not_found", { title: "Not found" }),
        ),
        goalListHandler([]),
      ],
    });

    expect(await screen.findByText("Not found")).toBeInTheDocument();
  });
});
