import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import { getListTasksMockHandler } from "../../api/generated/tasks/tasks.msw";
import { identity, page, project, task, uuid } from "../../test/fixtures";
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
          requests.push(request.url);
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
