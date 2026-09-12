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
 * The S8 definition of done (#11): both lenses render a seeded project and
 * update live while an agent works via REST — no reloads anywhere.
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
  test("decomposition lens is the project home screen", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}`);

    await expect(page.getByRole("tab", { name: "Decomposition" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    for (const title of ["Graph epic", "First step", "Second step"]) {
      await expect(page.getByRole("button", { name: title })).toBeVisible();
    }
    // Two decomposition edges, no dependency edge in this lens.
    await expect(page.locator(".react-flow__edge")).toHaveCount(2);
  });

  test("toggling lenses swaps the edge set and keeps selection", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}`);
    await expect(page.getByRole("button", { name: "Graph epic" })).toBeVisible();

    // Select a node (body click), then switch lens: selection survives
    // because node ids are task ids in both lenses.
    await page.locator(`[data-id="${lens.id}"]`).click();
    await expect(page).toHaveURL(new RegExp(`selected=${lens.id}`));

    await page.getByRole("tab", { name: "Dependency flow" }).click();
    await expect(page.locator(".react-flow__edge")).toHaveCount(1);
    await expect(page).toHaveURL(/lens=flow/);
    await expect(page).toHaveURL(new RegExp(`selected=${lens.id}`));
    await expect(page.locator(`[data-id="${lens.id}"] article`)).toHaveAttribute(
      "data-selected",
      "true",
    );
  });

  test("?lens=flow deep link renders the dependency DAG start → end", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}?lens=flow`);

    await expect(page.getByRole("tab", { name: "Dependency flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.locator(".react-flow__edge")).toHaveCount(1);

    // Prerequisite sits left of its dependent.
    const first = await page.locator(`[data-id="${lens.id}"]`).boundingBox();
    const second = await page.locator(`[data-id="${flow.id}"]`).boundingBox();
    expect(first!.x).toBeLessThan(second!.x);
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
    await expect(page.locator(".react-flow__edge")).toHaveCount(1, { timeout: 10_000 });
  });
});
