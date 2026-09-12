import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { getListKnowledgeMockHandler } from "../../api/generated/knowledge/knowledge.msw";
import { getGetProjectMockHandler } from "../../api/generated/projects/projects.msw";
import { knowledgeItem, page, project } from "../../test/fixtures";
import { problemResponse } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("Project knowledge screen", () => {
  const proj = project({ name: "Alpha" });

  const convention = knowledgeItem({
    project_id: proj.id,
    scope: "project",
    type: "note",
    title: "House conventions",
    content: "Small commits, spec first.",
  });
  const decision = knowledgeItem({
    project_id: proj.id,
    scope: "task",
    task_id: "00000000-0000-4000-8000-0000000000aa",
    type: "decision",
    title: "Graph library decision",
    content: "React Flow with dagre.",
  });
  const link = knowledgeItem({
    project_id: proj.id,
    scope: "session",
    type: "link",
    title: "The PR",
    content: "https://example.test/pr/1",
  });

  function renderKnowledge(path = "", items = [convention, decision, link]) {
    return renderRoute(`/projects/${proj.id}/knowledge${path}`, {
      handlers: [getGetProjectMockHandler(proj), getListKnowledgeMockHandler(page(items))],
    });
  }

  it("lists every knowledge item with type, scope, and provenance", async () => {
    renderKnowledge();

    expect(await screen.findByText("House conventions")).toBeInTheDocument();
    expect(screen.getByText("Graph library decision")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("3 of 3 items");

    // Links render as anchors, provenance links point at the task.
    expect(screen.getByRole("link", { name: "https://example.test/pr/1" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "From task" })).toHaveAttribute(
      "href",
      `/projects/${proj.id}/tasks/${decision.task_id}`,
    );
  });

  it("searches across title and content, case-insensitively", async () => {
    renderKnowledge();
    await screen.findByText("House conventions");

    await userEvent.type(screen.getByLabelText("Search"), "REACT FLOW");

    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("1 of 3 items"));
    expect(screen.getByText("Graph library decision")).toBeInTheDocument();
    expect(screen.queryByText("House conventions")).not.toBeInTheDocument();
  });

  it("honors a ?q= deep link", async () => {
    renderKnowledge("?q=conventions");

    expect(await screen.findByText("House conventions")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("1 of 3 items");
  });

  it("sends scope and type filters to the server", async () => {
    const urls: string[] = [];
    renderRoute(`/projects/${proj.id}/knowledge?scope=project&type=note`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListKnowledgeMockHandler(({ request }) => {
          urls.push(request.url);
          return page([convention]);
        }),
      ],
    });

    expect(await screen.findByText("House conventions")).toBeInTheDocument();
    const params = new URL(urls.at(-1)!).searchParams;
    expect(params.get("scope")).toBe("project");
    expect(params.get("type")).toBe("note");
  });

  it("drains every page before searching", async () => {
    const fromPageTwo = knowledgeItem({
      project_id: proj.id,
      title: "Deep cut",
      content: "buried on page two",
    });
    renderRoute(`/projects/${proj.id}/knowledge`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListKnowledgeMockHandler(({ request }) =>
          new URL(request.url).searchParams.get("cursor") === null
            ? page([convention], "cursor-2")
            : page([fromPageTwo]),
        ),
      ],
    });

    expect(await screen.findByText("Deep cut")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("2 of 2 items");
  });

  it("shows empty and error states", async () => {
    renderKnowledge("", []);
    expect(await screen.findByText("No knowledge yet")).toBeInTheDocument();

    renderRoute(`/projects/${proj.id}/knowledge`, {
      handlers: [
        getGetProjectMockHandler(proj),
        http.get(`/api/v1/projects/${proj.id}/knowledge`, () =>
          problemResponse(500, "internal", { title: "Storage exploded" }),
        ),
      ],
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Storage exploded");
  });
});
