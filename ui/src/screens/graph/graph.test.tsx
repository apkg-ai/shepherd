import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import { getListProjectRelationsMockHandler } from "../../api/generated/relations/relations.msw";
import { getListTasksMockHandler } from "../../api/generated/tasks/tasks.msw";
import { identity, page, project, relation, task } from "../../test/fixtures";
import { problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("Graph screen", () => {
  const proj = project({ name: "Alpha" });

  const parent = task({ project_id: proj.id, title: "Epic", status: "done" });
  const step1 = task({ project_id: proj.id, title: "Step one", status: "done" });
  const step2 = task({
    project_id: proj.id,
    title: "Step two",
    status: "in_progress",
    assignee: identity({ label: "Agent A" }),
    attempt_count: 2,
  });

  // Epic decomposes into both steps; step two depends on step one.
  const relations = [
    relation({ type: "decomposition", source_task_id: parent.id, target_task_id: step1.id }),
    relation({ type: "decomposition", source_task_id: parent.id, target_task_id: step2.id }),
    relation({ type: "depends_on", source_task_id: step2.id, target_task_id: step1.id }),
  ];

  function renderGraph(path = "") {
    return renderRoute(`/projects/${proj.id}${path}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([parent, step1, step2])),
        getListProjectRelationsMockHandler(page(relations)),
      ],
    });
  }

  it("renders the decomposition lens by default with the status language", async () => {
    renderGraph();

    expect(await screen.findByText("Epic")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Decomposition" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // Decomposition edges only: parent→child twice, no dependency edge.
    expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(2);

    // Status language on the node: in_progress carries the claimant chip
    // and the attempt badge.
    const node = screen.getByText("Step two").closest("article")!;
    expect(node).toHaveAttribute("data-status", "in_progress");
    expect(within(node).getByText("Agent A")).toBeInTheDocument();
    expect(within(node).getByText("2 attempts")).toBeInTheDocument();
  });

  it("toggles to the dependency flow lens", async () => {
    renderGraph();

    await screen.findByText("Epic");
    await userEvent.click(screen.getByRole("tab", { name: "Dependency flow" }));

    // Only the depends_on edge remains, rendered start → end.
    await waitFor(() => expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(1));
    expect(screen.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // All tasks stay on the canvas, connected or not.
    expect(screen.getByText("Epic")).toBeInTheDocument();
  });

  it("honors a ?lens=flow deep link", async () => {
    renderGraph("?lens=flow");

    await screen.findByText("Epic");
    expect(screen.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(1);
  });

  it("mirrors node selection to ?selected= so it survives a lens switch", async () => {
    const { router } = renderGraph();

    const title = await screen.findByText("Step two");
    // fireEvent, not userEvent: userEvent's mousedown carries a null
    // event.view in jsdom, which crashes d3-zoom's pan handler.
    fireEvent.click(title.closest("article")!);
    await waitFor(() =>
      expect(new URLSearchParams(router.state.location.search).get("selected")).toBe(step2.id),
    );
    expect(title.closest("article")).toHaveAttribute("data-selected", "true");

    await userEvent.click(screen.getByRole("tab", { name: "Dependency flow" }));
    await waitFor(() =>
      expect(screen.getByText("Step two").closest("article")).toHaveAttribute(
        "data-selected",
        "true",
      ),
    );
  });

  it("links node titles to task detail", async () => {
    renderGraph();

    const title = await screen.findByRole("link", { name: "Epic" });
    expect(title).toHaveAttribute("href", `/projects/${proj.id}/tasks/${parent.id}`);
  });

  it("marks real flow boundaries but not edge-less tasks", async () => {
    const start = task({ project_id: proj.id, title: "Starter", graph_role: ["start"] });
    const milestone = task({
      project_id: proj.id,
      title: "Milestone",
      graph_role: ["milestone"],
    });
    const isolated = task({
      project_id: proj.id,
      title: "Floater",
      status: "cancelled",
      graph_role: ["start", "end"],
    });
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([start, milestone, isolated])),
        getListProjectRelationsMockHandler(page([])),
      ],
    });

    const startNode = (await screen.findByText("Starter")).closest("article")!;
    expect(within(startNode).getByText("start")).toBeInTheDocument();
    expect(
      within(screen.getByText("Milestone").closest("article")!).getByText("milestone"),
    ).toBeInTheDocument();

    // start+end together is the derived default for edge-less tasks — no chips.
    const floater = screen.getByText("Floater").closest("article")!;
    expect(within(floater).queryByText("start")).not.toBeInTheDocument();
    expect(within(floater).queryByText("end")).not.toBeInTheDocument();
    expect(floater).toHaveAttribute("data-status", "cancelled");
  });

  it("shows an empty state with a create link when the project has no tasks", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([])),
        getListProjectRelationsMockHandler(page([])),
      ],
    });

    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Create the first task" })).toHaveAttribute(
      "href",
      `/projects/${proj.id}/tasks/new`,
    );
  });

  it("surfaces feed errors with a retry", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([])),
        http.get(`/api/v1/projects/${proj.id}/relations`, () =>
          problemResponse(500, "internal", { title: "Storage exploded" }),
        ),
      ],
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("Storage exploded");
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("drains every page of the task and relation feeds", async () => {
    const more = task({ title: "From page two" });
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(({ request }) =>
          new URL(request.url).searchParams.get("cursor") === null
            ? page([parent, step1, step2], "cursor-2")
            : page([more]),
        ),
        getListProjectRelationsMockHandler(page(relations)),
      ],
    });

    expect(await screen.findByText("From page two")).toBeInTheDocument();
    expect(screen.getByText("Epic")).toBeInTheDocument();
  });
});
