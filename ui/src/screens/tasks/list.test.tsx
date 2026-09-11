import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import {
  getGetTaskMockHandler,
  getListTasksMockHandler,
} from "../../api/generated/tasks/tasks.msw";
import { getListTaskRelationsMockHandler } from "../../api/generated/relations/relations.msw";
import { identity, page, project, relation, task, uuid } from "../../test/fixtures";
import { problemResponse, server } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("Project task list", () => {
  const proj = project({ name: "Alpha" });

  function renderList(extra: Parameters<typeof renderRoute>[1] = {}, path = "") {
    return renderRoute(`/projects/${proj.id}${path}`, {
      handlers: [getGetProjectMockHandler(proj), ...(extra.handlers ?? [])],
    });
  }

  it("renders rows with the status language", async () => {
    const inProgress = task({
      title: "Claimed work",
      status: "in_progress",
      assignee: identity({ label: "Agent A" }),
      attempt_count: 2,
    });
    const blocked = task({ title: "Stuck work", status: "blocked" });
    renderList({
      handlers: [getListTasksMockHandler(page([inProgress, blocked]))],
    });

    expect(await screen.findByText("Claimed work")).toBeInTheDocument();
    expect(screen.getByText("In progress")).toHaveAttribute("data-status", "in_progress");
    expect(screen.getByText("Agent A")).toBeInTheDocument();
    expect(screen.getByText("2 attempts")).toBeInTheDocument();
    expect(screen.getByText("Blocked")).toHaveAttribute("data-status", "blocked");
    expect(screen.getByRole("link", { name: "Stuck work" })).toHaveAttribute(
      "href",
      `/projects/${proj.id}/tasks/${blocked.id}`,
    );
  });

  it("filters by status and type via search params", async () => {
    const requests: string[] = [];
    renderList({
      handlers: [
        getListTasksMockHandler(({ request }) => {
          // The sidebar ReviewBadge queries with limit=25 — count list fetches only.
          if (!new URL(request.url).searchParams.has("limit")) {
            requests.push(request.url);
          }
          return page([task({ title: "Filtered" })]);
        }),
      ],
    });

    await screen.findByText("Filtered");
    await userEvent.selectOptions(screen.getByLabelText("Status"), "in_review");
    await screen.findByText("Filtered");
    await userEvent.selectOptions(screen.getByLabelText("Type"), "research");
    await waitFor(() => expect(requests.length).toBe(3));

    const last = new URL(requests.at(-1)!);
    expect(last.searchParams.get("status")).toBe("in_review");
    expect(last.searchParams.get("type")).toBe("research");
  });

  it("initializes filters from the URL", async () => {
    let url = "";
    renderList(
      {
        handlers: [
          getListTasksMockHandler(({ request }) => {
            url = request.url;
            return page([]);
          }),
        ],
      },
      "?status=done",
    );
    await screen.findByText("No tasks match these filters");
    expect(new URL(url).searchParams.get("status")).toBe("done");
    expect(screen.getByLabelText("Status")).toHaveValue("done");
  });

  it("pages through the cursor", async () => {
    server.use(
      getListTasksMockHandler(({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        return cursor === "c2"
          ? page([task({ title: "Second page" })])
          : page([task({ title: "First page" })], "c2");
      }),
    );
    renderList();

    expect(await screen.findByText("First page")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Second page")).toBeInTheDocument();
    expect(screen.getByText("First page")).toBeInTheDocument();
  });

  it("shows the empty state with a create link", async () => {
    renderList({ handlers: [getListTasksMockHandler(page([]))] });
    expect(await screen.findByText("No tasks yet")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Create the first task" })).toHaveAttribute(
      "href",
      `/projects/${proj.id}/tasks/new`,
    );
  });

  it("shows an error state on 500", async () => {
    renderRoute(`/projects/${uuid()}`, {
      handlers: [
        http.get("/api/v1/projects/:projectId/tasks", () =>
          problemResponse(500, "internal-error", { title: "Storage failure" }),
        ),
      ],
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Storage failure");
  });
});

describe("Task hierarchy", () => {
  const proj = project({ name: "Alpha" });

  /** Relations resolver keyed by task id; everything else gets none. */
  function relationsFor(map: Record<string, ReturnType<typeof relation>[]>) {
    return getListTaskRelationsMockHandler(({ params }) => ({
      items: map[String(params["taskId"])] ?? [],
    }));
  }

  function seedFamily() {
    const parent = task({ title: "Parent epic", status: "ready" });
    const child = task({ title: "Child one", status: "approved" });
    const dep = task({ title: "Prerequisite", status: "in_progress" });
    const edge = relation({
      type: "decomposition",
      source_task_id: parent.id,
      target_task_id: child.id,
    });
    const depEdge = relation({
      type: "depends_on",
      source_task_id: child.id,
      target_task_id: dep.id,
    });
    return { parent, child, dep, edge, depEdge };
  }

  it("groups children under parents with subtask counts and collapse", async () => {
    const { parent, child, dep, edge, depEdge } = seedFamily();
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([parent, child, dep])),
        relationsFor({
          [parent.id]: [edge],
          [child.id]: [edge, depEdge],
          [dep.id]: [depEdge],
        }),
      ],
    });

    expect(await screen.findByText("Parent epic")).toBeInTheDocument();
    expect(await screen.findByText("1 subtask")).toBeInTheDocument();
    expect(screen.getByText("Child one")).toBeInTheDocument();

    // Child waits on an in_progress dependency whose status is in view.
    expect(await screen.findByText("waits on 1")).toBeInTheDocument();

    // Collapse hides the child, expand brings it back.
    const toggle = screen.getByRole("button", { name: "Collapse Parent epic" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(toggle);
    expect(screen.queryByText("Child one")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Expand Parent epic" }));
    expect(screen.getByText("Child one")).toBeInTheDocument();
  });

  it("shows a parent breadcrumb for children whose parent is not loaded", async () => {
    const child = task({ title: "Stray child" });
    const farParent = task({ title: "Far away parent" });
    const edge = relation({
      type: "decomposition",
      source_task_id: farParent.id,
      target_task_id: child.id,
    });
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([child])),
        relationsFor({ [child.id]: [edge] }),
        getGetTaskMockHandler(farParent),
      ],
    });

    expect(await screen.findByText("Stray child")).toBeInTheDocument();
    expect(await screen.findByText("↳ Far away parent")).toBeInTheDocument();
  });

  it("flattens with breadcrumbs when filters are active", async () => {
    const { parent, child, edge } = seedFamily();
    renderRoute(`/projects/${proj.id}?status=approved`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(({ request }) => {
          const url = new URL(request.url);
          if (url.searchParams.get("status") === "approved") {
            return page([parent, child]);
          }
          return page([]);
        }),
        relationsFor({ [parent.id]: [edge], [child.id]: [edge] }),
        getGetTaskMockHandler(parent),
      ],
    });

    // Both rows flat (no collapse control), the child carrying its crumb.
    expect(await screen.findByText("Child one")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Collapse|Expand/ })).not.toBeInTheDocument();
    expect(await screen.findByText("↳ Parent epic")).toBeInTheDocument();
  });

  it("falls back to a dependency count when target statuses are unknown", async () => {
    const dependent = task({ title: "Needs things" });
    const edge1 = relation({
      type: "depends_on",
      source_task_id: dependent.id,
      target_task_id: "00000000-0000-4000-8000-aaaaaaaaaaaa",
    });
    const edge2 = relation({
      type: "depends_on",
      source_task_id: dependent.id,
      target_task_id: "00000000-0000-4000-8000-bbbbbbbbbbbb",
    });
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([dependent])),
        relationsFor({ [dependent.id]: [edge1, edge2] }),
      ],
    });
    expect(await screen.findByText("2 dependencies")).toBeInTheDocument();
  });
});
