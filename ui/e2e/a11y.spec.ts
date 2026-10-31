import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test } from "./helpers/coverage";
import { login } from "./helpers/auth";

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"];
const DISABLED_RULES = ["frame-tested", "hidden-content"];

async function auditBothThemes(page: Page, name: string) {
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((t) => {
      localStorage.setItem("shepherd-theme", t);
      document.documentElement.dataset["theme"] = t;
    }, theme);
    await page.waitForTimeout(150);
    const results = await new AxeBuilder({ page })
      .withTags(TAGS)
      .disableRules(DISABLED_RULES)
      .analyze();
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

test("login screen passes WCAG AA in both themes", async ({ page }) => {
  await auditRoute(page, "/#/login", "login");
});

test("project list passes WCAG AA in both themes", async ({ page }) => {
  await login(page);
  await auditRoute(page, "/#/", "project-list");
});

test("not-found screen passes WCAG AA in both themes", async ({ page }) => {
  await auditRoute(page, "/#/definitely-not-a-route", "not-found");
});

/**
 * Hierarchy screen a11y audits.
 * Seeds data via the API, then audits each route in both themes.
 * Runs after the hierarchy.spec flow (tests share the server).
 */
test.describe("hierarchy a11y", () => {
  let projectId: string;
  let goalId: string;
  let epicId: string;
  let taskId: string;

  test.beforeAll(async () => {
    const baseURL = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:7543";
    const token = process.env["SHEPHERD_OWNER_TOKEN"]!;
    const headers: Record<string, string> = {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    };

    const post = async (path: string, body: Record<string, unknown>) => {
      const res = await fetch(`${baseURL}${path}`, {
        method: "POST",
        headers: { ...headers, "Idempotency-Key": crypto.randomUUID() },
        body: JSON.stringify(body),
      });
      if (!res.ok) throw new Error(`Seed failed: ${res.status} ${await res.text()}`);
      return res.json();
    };

    const project = await post("/api/v1/projects", { name: "A11y Test" });
    projectId = project.id;

    const goal = await post(`/api/v1/projects/${projectId}/goals`, { title: "A11y Goal" });
    goalId = goal.id;

    const epic = await post(`/api/v1/projects/${projectId}/epics`, {
      goal_id: goalId,
      title: "A11y Epic",
    });
    epicId = epic.id;

    const task = await post(`/api/v1/projects/${projectId}/tasks`, {
      epic_id: epicId,
      title: "A11y Task",
      type_key: "code",
    });
    taskId = task.id;
  });

  test("project overview passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}`, "project-overview");
  });

  test("goal detail passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/goals/${goalId}`, "goal-detail");
  });

  test("epic detail passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/epics/${epicId}`, "epic-detail");
  });

  test("task detail passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/tasks/${taskId}`, "task-detail");
  });

  test("goal form passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/goals/new`, "goal-form");
  });

  test("epic form passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/goals/${goalId}/epics/new`, "epic-form");
  });

  test("task form passes WCAG AA", async ({ page }) => {
    await login(page);
    await auditRoute(page, `/#/projects/${projectId}/epics/${epicId}/tasks/new`, "task-form");
  });
});
