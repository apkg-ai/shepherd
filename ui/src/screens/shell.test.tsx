import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  getGetProjectMockHandler,
  getListProjectsMockHandler,
} from "../api/generated/projects/projects.msw";
import { getListProjectRelationsMockHandler } from "../api/generated/relations/relations.msw";
import { getListTasksMockHandler } from "../api/generated/tasks/tasks.msw";
import { page, project, task } from "../test/fixtures";
import { renderRoute } from "../test/test-utils";

describe("App shell", () => {
  it("navigates registry → graph → tasks → review → settings", async () => {
    const proj = project({ name: "Alpha" });
    renderRoute("/", {
      handlers: [
        getListProjectsMockHandler(page([proj])),
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([task({ title: "A task" })])),
        getListProjectRelationsMockHandler(page([])),
      ],
    });

    // Registry → project (lands on the graph, the project's home screen)
    const main = screen.getByRole("main");
    await userEvent.click(await within(main).findByRole("link", { name: "Alpha" }));
    expect(await screen.findByRole("tab", { name: "Decomposition" })).toBeInTheDocument();

    // Project nav is visible with the project name
    const nav = screen.getByRole("navigation", { name: "Project" });
    expect(within(nav).getByText("Alpha")).toBeInTheDocument();

    // → Tasks table
    await userEvent.click(within(nav).getByRole("link", { name: "Tasks" }));
    expect(await screen.findByText("A task")).toBeInTheDocument();

    // → Review
    await userEvent.click(within(nav).getByRole("link", { name: /^Review/ }));
    expect(await screen.findByRole("tab", { name: /^In review/ })).toBeInTheDocument();

    // → Settings
    await userEvent.click(within(nav).getByRole("link", { name: "Settings" }));
    expect(await screen.findByRole("button", { name: "Save settings" })).toBeInTheDocument();

    // Brand goes home; nav disappears without a project
    await userEvent.click(screen.getByRole("link", { name: "Shepherd" }));
    expect(
      await within(screen.getByRole("main")).findByRole("link", { name: "Alpha" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Project" })).not.toBeInTheDocument();
  });

  it("keeps a single h1 across screens (#21 regression guard)", async () => {
    renderRoute("/", {
      handlers: [getListProjectsMockHandler(page([]))],
    });
    await screen.findByText("Register your first project");
    expect(document.querySelectorAll("h1")).toHaveLength(1);
  });

  it("renders NotFound for unknown routes", async () => {
    renderRoute("/nowhere/at/all");
    expect(await screen.findByText("Page not found")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to projects" })).toBeInTheDocument();
  });
});

describe("Sidebar", () => {
  const proj = project({ name: "Alpha" });

  it("shows the project switcher, review badge, and theme toggle", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListProjectsMockHandler(page([proj])),
        getListTasksMockHandler(({ request }) => {
          const url = new URL(request.url);
          return url.searchParams.get("status") === "in_review"
            ? page([task({ status: "in_review" }), task({ status: "in_review" })])
            : page([]);
        }),
        getListProjectRelationsMockHandler(page([])),
      ],
    });

    const switcher = await screen.findByRole("navigation", { name: "Projects" });
    expect(await within(switcher).findByRole("link", { name: "Alpha" })).toHaveAttribute(
      "aria-current",
      "true",
    );
    expect(within(switcher).getByRole("link", { name: "All projects" })).toBeInTheDocument();

    // Review badge counts the in_review page
    const reviewLink = await screen.findByRole("link", { name: /^Review/ });
    expect(await within(reviewLink).findByText("2")).toBeInTheDocument();

    // Theme toggle lives in the sidebar footer
    expect(screen.getByRole("group", { name: "Theme" })).toBeInTheDocument();
  });

  it("hedges the review badge past one page", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListProjectsMockHandler(page([proj])),
        getListTasksMockHandler(({ request }) => {
          const url = new URL(request.url);
          return url.searchParams.get("status") === "in_review"
            ? page([task({ status: "in_review" })], "next-cursor")
            : page([]);
        }),
        getListProjectRelationsMockHandler(page([])),
      ],
    });
    const reviewLink = await screen.findByRole("link", { name: /^Review/ });
    expect(await within(reviewLink).findByText("1+")).toBeInTheDocument();
  });

  it("hides the review badge when nothing is in review", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListProjectsMockHandler(page([proj])),
        getListTasksMockHandler(page([])),
        getListProjectRelationsMockHandler(page([])),
      ],
    });
    const reviewLink = await screen.findByRole("link", { name: /^Review/ });
    expect(reviewLink).toHaveTextContent(/^Review$/);
  });
});

describe("Combined review badge", () => {
  const proj = project({ name: "Alpha" });

  it("counts work in review and proposals together", async () => {
    renderRoute(`/projects/${proj.id}`, {
      handlers: [
        getGetProjectMockHandler(proj),
        getListProjectsMockHandler(page([proj])),
        getListTasksMockHandler(({ request }) => {
          const status = new URL(request.url).searchParams.get("status");
          if (status === "in_review") {
            return page([task({ status: "in_review" }), task({ status: "in_review" })]);
          }
          if (status === "proposed") return page([task({ status: "proposed" })]);
          return page([]);
        }),
        getListProjectRelationsMockHandler(page([])),
      ],
    });
    const reviewLink = await screen.findByRole("link", { name: /^Review/ });
    expect(await within(reviewLink).findByText("3")).toBeInTheDocument();
  });
});
