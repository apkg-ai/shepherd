import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  getGetProjectMockHandler,
  getListProjectsMockHandler,
} from "../api/generated/projects/projects.msw";
import { getListTasksMockHandler } from "../api/generated/tasks/tasks.msw";
import { page, project, task } from "../test/fixtures";
import { renderRoute } from "../test/test-utils";

describe("App shell", () => {
  it("navigates registry → project tasks → review → settings", async () => {
    const proj = project({ name: "Alpha" });
    renderRoute("/", {
      handlers: [
        getListProjectsMockHandler(page([proj])),
        getGetProjectMockHandler(proj),
        getListTasksMockHandler(page([task({ title: "A task" })])),
      ],
    });

    // Registry → project
    await userEvent.click(await screen.findByRole("link", { name: "Alpha" }));
    expect(await screen.findByText("A task")).toBeInTheDocument();

    // Project nav is visible with the project name
    const nav = screen.getByRole("navigation", { name: "Project" });
    expect(within(nav).getByText("Alpha")).toBeInTheDocument();

    // → Review
    await userEvent.click(within(nav).getByRole("link", { name: "Review" }));
    expect(await screen.findByRole("tab", { name: "In review" })).toBeInTheDocument();

    // → Settings
    await userEvent.click(within(nav).getByRole("link", { name: "Settings" }));
    expect(await screen.findByRole("button", { name: "Save settings" })).toBeInTheDocument();

    // Brand goes home; nav disappears without a project
    await userEvent.click(screen.getByRole("link", { name: "Shepherd" }));
    expect(await screen.findByRole("link", { name: "Alpha" })).toBeInTheDocument();
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
