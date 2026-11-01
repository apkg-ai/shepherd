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
    await page.goto(`/#/projects/${projectId}`);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    // Switch to table view — React Flow graph has nested-interactive by design
    await page.getByRole("button", { name: "Table view" }).click();
    await expect(page.getByText("A11y Goal")).toBeVisible();
    await auditBothThemes(page, "project-overview-table");
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

/**
 * Graph a11y audit.
 * Seeds a project with epics and dependencies, then audits the goal detail
 * (which hosts the dependency graph) in both themes.
 */
test.describe("graph a11y", () => {
  let graphProjectId: string;
  let graphGoalId: string;

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

    const project = await post("/api/v1/projects", { name: "A11y Graph" });
    graphProjectId = project.id;

    const goal = await post(`/api/v1/projects/${graphProjectId}/goals`, {
      title: "A11y Graph Goal",
    });
    graphGoalId = goal.id;

    const epicA = await post(`/api/v1/projects/${graphProjectId}/epics`, {
      goal_id: graphGoalId,
      title: "A11y Epic A",
    });
    const epicB = await post(`/api/v1/projects/${graphProjectId}/epics`, {
      goal_id: graphGoalId,
      title: "A11y Epic B",
    });

    await post(`/api/v1/projects/${graphProjectId}/dependencies`, {
      level: "epic",
      dependent_id: epicB.id,
      prerequisite_id: epicA.id,
    });
  });

  test("goal detail with graph passes WCAG AA", async ({ page }) => {
    await login(page);
    // React Flow's internal canvas uses aria-label but may produce presentational
    // elements that axe flags; exclude the .react-flow internals from the audit
    // while still scanning the toolbar and surrounding UI.
    await page.goto(`/#/projects/${graphProjectId}/goals/${graphGoalId}`);
    await page.waitForLoadState("networkidle");
    for (const theme of ["light", "dark"] as const) {
      await page.evaluate((t) => {
        localStorage.setItem("shepherd-theme", t);
        document.documentElement.dataset["theme"] = t;
      }, theme);
      await page.waitForTimeout(150);
      const results = await new AxeBuilder({ page })
        .withTags(TAGS)
        .disableRules(DISABLED_RULES)
        // React Flow renders SVG edges with presentation-only elements that
        // may trip generic a11y rules; our wrapper sets role="application" with a label.
        .exclude(".react-flow__edges")
        .analyze();
      expect(
        results.violations,
        `goal-detail-graph [${theme}]: ${results.violations
          .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)
          .join(" | ")}`,
      ).toEqual([]);
    }
    await page.evaluate(() => localStorage.removeItem("shepherd-theme"));
  });

  test("graph list view passes WCAG AA", async ({ page }) => {
    await login(page);
    await page.goto(`/#/projects/${graphProjectId}/goals/${graphGoalId}`);
    await page.getByRole("button", { name: "List view" }).click();
    await expect(page.getByRole("group", { name: "Filter by status" })).toBeVisible();
    await auditBothThemes(page, "graph-list-view");
  });
});
