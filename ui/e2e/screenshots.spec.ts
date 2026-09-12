import { mkdirSync } from "node:fs";
import type { Page } from "@playwright/test";
import { expect, test } from "./helpers/coverage";
import {
  createKnowledge,
  createProject,
  createRelation,
  createTask,
  reportSession,
  seedInReviewTask,
  type SeededProject,
} from "./helpers/api";

/**
 * Not assertions — captures. Plain page.screenshot of every key state in both
 * themes into e2e-artifacts/screens/ (gitignored) so a human or agent can
 * review the actual rendered design. No toHaveScreenshot, no snapshots.
 */

const DIR = "e2e-artifacts/screens";

let project: SeededProject;
let detailTaskId: string;

test.beforeAll(async () => {
  mkdirSync(DIR, { recursive: true });
  project = await createProject(`Showcase ${Date.now()}`, {
    description: "Seeded showcase project for the visual pass",
  });
  const epic = await createTask(project.id, "Ship the S8 graph", {
    description: "Parent epic for the showcase — decomposed into the pieces below.",
    metadata: { milestone: "v1", theme: "graph" },
  });
  const lens = await createTask(project.id, "Decomposition lens", {
    description: "Tree lens of parent/child structure",
  });
  const flow = await createTask(project.id, "Dependency flow lens", {
    type: "research",
  });
  await createRelation(project.id, epic.id, "decomposition", lens.id);
  await createRelation(project.id, epic.id, "decomposition", flow.id);
  await createRelation(project.id, flow.id, "depends_on", lens.id);

  // Attempt history: one failure, one success → in review.
  await reportSession(project.id, lens.id, "failed", {
    summary: "Spike with the wrong layout engine",
    failureReason: "Layout collapsed on cycles",
  });
  await reportSession(project.id, lens.id, "succeeded", {
    summary: "Reworked on the bounded DAG layout",
  });
  detailTaskId = lens.id;

  await seedInReviewTask(project.id, "Review the palette");
  await createTask(project.id, "Proposed: live cursors", { status: "proposed" });
  await createKnowledge(project.id, {
    title: "Graph library decision",
    content: "React Flow + dagre under the minimal-deps rule.",
    type: "decision",
  });
  await createKnowledge(project.id, {
    title: "The PR",
    content: "https://example.test/pr/1",
    type: "link",
    scope: "task",
    taskId: detailTaskId,
  });
  const blocked = await createTask(project.id, "Blocked on infra");
  await fetch(
    `${process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:7543"}/api/v1/projects/${project.id}/tasks/${blocked.id}/block`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "Waiting for S9 infra" }),
    },
  );
});

async function capture(page: Page, path: string, name: string) {
  await page.goto(path);
  await page.waitForLoadState("networkidle");
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((t) => {
      localStorage.setItem("shepherd-theme", t);
      document.documentElement.dataset["theme"] = t;
    }, theme);
    await page.waitForTimeout(150);
    await page.screenshot({
      path: `${DIR}/${name}-${theme}.png`,
      fullPage: true,
    });
  }
  await page.evaluate(() => localStorage.removeItem("shepherd-theme"));
}

test("captures every key screen in both themes", async ({ page }) => {
  test.setTimeout(120_000);
  await capture(page, "/#/", "registry");
  await capture(page, `/#/projects/${project.id}`, "graph-tree-lens");
  await capture(page, `/#/projects/${project.id}?lens=flow`, "graph-flow-lens");
  await capture(page, `/#/projects/${project.id}/tasks`, "task-tree");
  await capture(page, `/#/projects/${project.id}/tasks?status=in_review`, "task-list-filtered");
  await capture(page, `/#/projects/${project.id}/knowledge`, "knowledge");
  await capture(page, `/#/projects/${project.id}/tasks/${detailTaskId}`, "task-detail");
  await capture(page, `/#/projects/${project.id}/tasks/new`, "task-form");
  await capture(page, `/#/projects/${project.id}/review`, "review-queue");
  await capture(page, `/#/projects/${project.id}/review?tab=proposals`, "review-proposals");
  await capture(page, `/#/projects/${project.id}/settings`, "settings");

  // Dialog states
  await page.goto("/#/");
  await page.getByRole("button", { name: "Register project" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.screenshot({ path: `${DIR}/dialog-register-light.png` });
});
