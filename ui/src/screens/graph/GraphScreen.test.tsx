import { http, HttpResponse } from "msw";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  sessionHandler,
  projectHandler,
  goalHandler,
  epicListHandler,
  goalGraphHandler,
  problemResponse,
} from "../../test/msw";
import {
  makeProject,
  makeGoal,
  makeEpic,
  makeGraphNode,
  makeDependency,
  makeGraph,
} from "../../test/fixtures";
import { renderRoute } from "../../test/test-utils";

const project = makeProject({ id: "p1", name: "Test Project" });
const goal = makeGoal({ id: "g1", project_id: "p1", title: "Test Goal" });

const nodeA = makeGraphNode({ id: "n1", title: "Node A", status: "open" });
const nodeB = makeGraphNode({ id: "n2", title: "Node B", status: "active" });
const dep = makeDependency({
  id: "d1",
  project_id: "p1",
  level: "epic",
  dependent_id: "n2",
  prerequisite_id: "n1",
});

const epics = [
  makeEpic({ id: "n1", project_id: "p1", goal_id: "g1", title: "Node A", status: "open" }),
  makeEpic({ id: "n2", project_id: "p1", goal_id: "g1", title: "Node B", status: "active" }),
];

const graphWithNodes = makeGraph({
  level: "epic",
  scope_id: "g1",
  nodes: [nodeA, nodeB],
  dependencies: [dep],
});

const emptyGraph = makeGraph({ level: "epic", scope_id: "g1", nodes: [], dependencies: [] });

const baseHandlers = [sessionHandler(), projectHandler(project), goalHandler(goal)];

describe("GraphScreen", () => {
  it("renders graph toolbar when graph data loads", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics), goalGraphHandler(graphWithNodes)],
    });

    const toolbar = await screen.findByRole("group", { name: "Graph controls" });
    expect(toolbar).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "List view" })).toBeInTheDocument();
  });

  it("shows empty state when graph has no nodes", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics), goalGraphHandler(emptyGraph)],
    });

    expect(await screen.findByText("No items to display")).toBeInTheDocument();
  });

  it("toggles between graph and list views", async () => {
    const user = userEvent.setup();
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics), goalGraphHandler(graphWithNodes)],
    });

    const toolbar = await screen.findByRole("group", { name: "Graph controls" });
    expect(toolbar).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "List view" }));

    expect(await screen.findByText("Node A")).toBeInTheDocument();
    expect(screen.getByText("Node B")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Graph view" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Graph view" }));

    expect(await screen.findByRole("button", { name: "List view" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom out" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset layout" })).toBeInTheDocument();
  });

  it("shows graph_too_large fallback when server rejects", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [
        ...baseHandlers,
        epicListHandler(epics),
        http.get("*/api/v1/projects/:projectId/goals/:goalId/graph", () =>
          HttpResponse.json(
            {
              type: "urn:shepherd:error:graph_too_large",
              title: "Graph too large",
              status: 422,
            },
            { status: 422, headers: { "Content-Type": "application/problem+json" } },
          ),
        ),
      ],
    });

    expect(
      await screen.findByText("This graph is too large to display. Use the list view instead."),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText("Node A")).toBeInTheDocument();
  });

  it("shows error state when graph query fails", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [
        ...baseHandlers,
        epicListHandler(epics),
        http.get("*/api/v1/projects/:projectId/goals/:goalId/graph", () =>
          problemResponse(500, "internal-error", { title: "Graph Error" }),
        ),
      ],
    });

    expect(await screen.findByText("Graph Error")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("add dependency button is disabled when no node selected", async () => {
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics), goalGraphHandler(graphWithNodes)],
    });

    const addBtn = await screen.findByRole("button", { name: "Add dependency" });
    expect(addBtn).toBeDisabled();
  });

  it("list view shows status filter and cards", async () => {
    const user = userEvent.setup();
    renderRoute("/projects/p1/goals/g1", {
      handlers: [...baseHandlers, epicListHandler(epics), goalGraphHandler(graphWithNodes)],
    });

    await screen.findByRole("group", { name: "Graph controls" });
    await user.click(screen.getByRole("button", { name: "List view" }));

    expect(await screen.findByRole("group", { name: "Filter by status" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Node A/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Node B/ })).toBeInTheDocument();
  });
});
