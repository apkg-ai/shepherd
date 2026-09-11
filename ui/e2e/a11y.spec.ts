import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test } from "./helpers/coverage";
import {
  createProject,
  createRelation,
  createTask,
  reportSession,
  seedInReviewTask,
  type SeededProject,
} from "./helpers/api";

/**
 * WCAG AA audit of every screen in both themes with axe-core — the rendered-
 * pixels layer on top of the token contrast gate (unit) and jsx-a11y (lint).
 * Any violation fails the suite; best-practice rules are deliberately out of
 * scope (AA is the bar).
 */

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

let project: SeededProject;
let detailTaskId: string;

test.beforeAll(async () => {
  project = await createProject(`A11y audit ${Date.now()}`, {
    description: "Seeded for the axe pass",
  });
  const epic = await createTask(project.id, "Audit epic", {
    metadata: { audited: true },
  });
  const child = await createTask(project.id, "Audit child");
  const dep = await createTask(project.id, "Audit dependency", { type: "research" });
  await createRelation(project.id, epic.id, "decomposition", child.id);
  await createRelation(project.id, child.id, "depends_on", dep.id);
  await reportSession(project.id, dep.id, "failed", {
    failureReason: "Simulated failure for badge states",
  });
  await seedInReviewTask(project.id, "Audit review item");
  await createTask(project.id, "Audit proposal", { status: "proposed" });
  detailTaskId = dep.id;
});

async function auditBothThemes(page: Page, name: string) {
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((t) => {
      localStorage.setItem("shepherd-theme", t);
      document.documentElement.dataset["theme"] = t;
    }, theme);
    await page.waitForTimeout(150);
    const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
    expect(
      results.violations,
      `${name} [${theme}]: ${results.violations
        .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)
        .join(" | ")}`,
    ).toEqual([]);
  }
  await page.evaluate(() => localStorage.removeItem("shepherd-theme"));
}

async function auditRoute(page: Page, path: string, name: string) {
  await page.goto(path);
  await page.waitForLoadState("networkidle");
  await auditBothThemes(page, name);
}

test.describe("axe WCAG AA audit", () => {
  test("project registry", async ({ page }) => {
    await auditRoute(page, "/#/", "registry");
  });

  test("task tree", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}`, "task tree");
  });

  test("filtered task list", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}?status=in_review`, "filtered list");
  });

  test("task detail", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}/tasks/${detailTaskId}`, "task detail");
  });

  test("task form", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}/tasks/new`, "task form");
  });

  test("review queue, both tabs", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}/review`, "review in_review");
    await auditRoute(page, `/#/projects/${project.id}/review?tab=proposals`, "review proposals");
  });

  test("project settings", async ({ page }) => {
    await auditRoute(page, `/#/projects/${project.id}/settings`, "settings");
  });

  test("register-project dialog", async ({ page }) => {
    await page.goto("/#/");
    await page.getByRole("button", { name: "Register project" }).click();
    await expect(page.getByRole("dialog", { name: "Register project" })).toBeVisible();
    await auditBothThemes(page, "register dialog");
  });

  test("add-relation dialog", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}/tasks/${detailTaskId}`);
    await page.getByRole("button", { name: "Add relation" }).click();
    await expect(page.getByRole("dialog", { name: "Add relation" })).toBeVisible();
    await auditBothThemes(page, "add-relation dialog");
  });

  test("reject dialog", async ({ page }) => {
    await page.goto(`/#/projects/${project.id}/review`);
    await page.getByRole("button", { name: "Reject" }).first().click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await auditBothThemes(page, "reject dialog");
  });
});
