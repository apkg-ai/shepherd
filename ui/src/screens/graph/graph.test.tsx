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

  it("defaults to the dependency flow lens showing only epics with Start/End", async () => {
    renderGraph();

    // Default lens is flow; only the "Epic" (a parent) is visible,
    // subtasks are filtered out. Virtual Start/End boundary nodes anchor the graph.
    expect(await screen.findByText("Epic")).toBeInTheDocument();
    expect(screen.getByText("Start")).toBeInTheDocument();
    expect(screen.getByText("End")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // Two boundary edges: Start→Epic and Epic→End (Epic has no prereqs and nothing depends on it).
    await waitFor(() =>
      expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(2),
    );
    // Epic carries a subtask count chip.
    const node = screen.getByText("Epic").closest("article")!;
    expect(within(node).getByText("2")).toBeInTheDocument();
  });

  it("shows the tree lens with expanded roots via ?lens=tree", async () => {
    renderGraph("?lens=tree");

    // Tree lens with auto-expanded root: children visible.
    expect(await screen.findByText("Epic")).toBeInTheDocument();
    expect(screen.getByText("Step one")).toBeInTheDocument();
    expect(screen.getByText("Step two")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Decomposition" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await waitFor(() =>
      expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(2),
    );
  });

  it("collapses everything in tree lens when ?expanded= is explicitly empty", async () => {
    renderGraph("?lens=tree&expanded=");

    expect(await screen.findByText("Epic")).toBeInTheDocument();
    expect(document.querySelectorAll(".react-flow__edge")).toHaveLength(0);
    const node = screen.getByText("Epic").closest("article")!;
    expect(
      within(node).getByRole("button", { name: "Expand 2 subtasks" }),
    ).toBeInTheDocument();
  });

  it("renders status language in tree lens", async () => {
    renderGraph("?lens=tree");

    expect(await screen.findByText("Step two")).toBeInTheDocument();
    const node = screen.getByText("Step two").closest("article")!;
    expect(node).toHaveAttribute("data-status", "in_progress");
    expect(within(node).getByText("Agent A")).toBeInTheDocument();
    expect(within(node).getByText("2 attempts")).toBeInTheDocument();
  });

  it("toggles between flow and tree lenses", async () => {
    renderGraph();

    // Start in flow lens (default).
    await screen.findByText("Epic");
    await userEvent.click(screen.getByRole("tab", { name: "Decomposition" }));

    // Tree lens shows all tasks (root auto-expanded).
    expect(await screen.findByText("Step one")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Decomposition" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("mirrors node selection to ?selected= so it survives a lens switch", async () => {
    // Start in tree lens so subtasks are visible.
    const { router } = renderGraph("?lens=tree");

    const title = await screen.findByTitle("Step two");
    fireEvent.click(title.closest("article")!);
    await waitFor(() =>
      expect(new URLSearchParams(router.state.location.search).get("selected")).toBe(step2.id),
    );
    expect(title.closest("article")).toHaveAttribute("data-selected", "true");

    // Switch to flow lens — selected task persists in the side panel even
    // though the node isn't visible (it's a subtask, not an epic).
    await userEvent.click(screen.getByRole("tab", { name: "Dependency flow" }));
    await waitFor(() =>
      expect(new URLSearchParams(router.state.location.search).get("selected")).toBe(step2.id),
    );
  });

  it("opens a side panel on selection with the task's context", async () => {
    // Tree lens: children visible and selectable.
    renderGraph("?lens=tree");

    fireEvent.click((await screen.findByTitle("Step two")).closest("article")!);

    const panel = await screen.findByRole("complementary", { name: "Selected task: Step two" });
    expect(within(panel).getByText("In progress")).toBeInTheDocument();
    expect(within(panel).getByText("Agent A")).toBeInTheDocument();
    expect(within(panel).getByText("2 attempts")).toBeInTheDocument();
    expect(within(panel).getByText("Parent")).toBeInTheDocument();
    expect(within(panel).getByRole("button", { name: "Epic" })).toBeInTheDocument();
    expect(within(panel).getByText("Depends on")).toBeInTheDocument();
    expect(within(panel).getByRole("link", { name: "Open full detail" })).toHaveAttribute(
      "href",
      `/projects/${proj.id}/tasks/${step2.id}`,
    );

    await userEvent.click(within(panel).getByRole("button", { name: "Step one" }));
    expect(
      await screen.findByRole("complementary", { name: "Selected task: Step one" }),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Close panel" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("complementary", { name: /Selected task/ }),
      ).not.toBeInTheDocument(),
    );
  });

  it("marks real flow boundaries in tree lens", async () => {
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
    // Use tree lens to see all tasks (these have no children → flow would hide them).
    renderRoute(`/projects/${proj.id}?lens=tree`, {
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
    // Use tree lens so all tasks (including non-epics) are visible.
    renderRoute(`/projects/${proj.id}?lens=tree`, {
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
