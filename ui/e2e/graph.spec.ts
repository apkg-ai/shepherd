import { expect, test } from "./helpers/coverage";
import {
  createProject,
  createRelation,
  createTask,
  reportSession,
  type SeededProject,
  type SeededTask,
} from "./helpers/api";

/**
 * The S9 graph (#48): the flow lens is the project home screen — epics only,
 * anchored by virtual Start/End boundary nodes, top→bottom. The tree lens
 * keeps the full decomposition with roots expanded. Both update live while
 * an agent works via REST — no reloads anywhere.
 */

let project: SeededProject;
let epic: SeededTask;
let lens: SeededTask;
let flow: SeededTask;

test.beforeAll(async () => {
  project = await createProject(`Graph ${Date.now()}`);
  epic = await createTask(project.id, "Graph epic");
  lens = await createTask(project.id, "First step");
  flow = await createTask(project.id, "Second step");
  await createRelation(project.id, epic.id, "decomposition", lens.id);
  await createRelation(project.id, epic.id, "decomposition", flow.id);
  // Second step depends on first: flow lens draws first → second.
  await createRelation(project.id, flow.id, "depends_on", lens.id);
});

test.describe("graph lenses", () => {
  test("flow lens is the project home screen", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}`);

    await expect(page.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // Epic-only: subtasks stay hidden until drill-down via the side panel.
    await expect(page.getByRole("button", { name: "Graph epic" })).toBeVisible();
    await expect(page.getByRole("button", { name: "First step" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Second step" })).toHaveCount(0);
    // Start → epic and epic → End boundary edges.
    await expect(page.locator(".react-flow__edge")).toHaveCount(2);
  });

  test("toggling lenses swaps the node set and keeps selection", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}`);
    await expect(page.getByRole("button", { name: "Graph epic" })).toBeVisible();

    // Select the epic (body click), then switch lens: selection survives
    // because node ids are task ids in both lenses.
    await page.locator(`[data-id="${epic.id}"]`).click();
    await expect(page).toHaveURL(new RegExp(`selected=${epic.id}`));

    await page.getByRole("tab", { name: "Decomposition" }).click();
    await expect(page).toHaveURL(/lens=tree/);
    await expect(page).toHaveURL(new RegExp(`selected=${epic.id}`));
    // Tree lens: roots expanded by default — the full decomposition shows.
    // (Node-wrapper locators: the open side panel lists the same titles.)
    for (const task of [epic, lens, flow]) {
      await expect(page.locator(`[data-id="${task.id}"]`)).toBeVisible();
    }
    await expect(page.locator(".react-flow__edge")).toHaveCount(2);
    await expect(page.locator(`[data-id="${epic.id}"] article`)).toHaveAttribute(
      "data-selected",
      "true",
    );
  });

  test("?lens=flow deep link renders the epic flow Start → End", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}?lens=flow`);

    await expect(page.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.locator(".react-flow__edge")).toHaveCount(2);

    // TB layout: Start above the epic, End below it.
    const start = await page.locator('[data-id="__start__"]').boundingBox();
    const epicBox = await page.locator(`[data-id="${epic.id}"]`).boundingBox();
    const end = await page.locator('[data-id="__end__"]').boundingBox();
    expect(start!.y).toBeLessThan(epicBox!.y);
    expect(epicBox!.y).toBeLessThan(end!.y);
  });

  test("selecting a node opens the side panel; full detail is a link away", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}`);

    await page.getByRole("button", { name: "Graph epic" }).click();
    const panel = page.getByRole("complementary", { name: "Selected task: Graph epic" });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("Subtasks")).toBeVisible();

    // Relation entries jump the selection without leaving the graph.
    await panel.getByRole("button", { name: "First step" }).click();
    await expect(
      page.getByRole("complementary", { name: "Selected task: First step" }),
    ).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`selected=${lens.id}`));

    await page.getByRole("link", { name: "Open full detail" }).click();
    await expect(page).toHaveURL(new RegExp(`/projects/${project.id}/tasks/${lens.id}`));
    await expect(page.getByRole("heading", { name: "First step" })).toBeVisible();
  });

  test("updates live while an agent works via REST", async ({ page }) => {
    const live = await createProject(`Graph live ${Date.now()}`);
    const start = await createTask(live.id, "Live start");
    await page.goto(`/#/projects/${live.id}?lens=flow`);
    // No epics in this project: the flow lens falls back to all tasks so
    // the default lens never renders a blank canvas.
    await expect(page.getByRole("button", { name: "Live start" })).toBeVisible();

    // An agent claims the task: the node restyles as in_progress with the
    // claimant chip — no reload.
    await reportSession(live.id, start.id, "succeeded");
    await expect(page.locator(`[data-id="${start.id}"] article`)).toHaveAttribute(
      "data-status",
      "in_review",
      { timeout: 10_000 },
    );

    // A new task + dependency appear as they're created.
    const next = await createTask(live.id, "Live next");
    await createRelation(live.id, next.id, "depends_on", start.id);
    await expect(page.getByRole("button", { name: "Live next" })).toBeVisible({
      timeout: 10_000,
    });
    // start → next plus the Start/End boundary edges around them.
    await expect(page.locator(".react-flow__edge")).toHaveCount(3, { timeout: 10_000 });
  });
});
