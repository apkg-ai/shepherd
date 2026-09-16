import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test } from "./helpers/coverage";

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

test("home screen passes WCAG AA in both themes", async ({ page }) => {
  await auditRoute(page, "/#/", "home");
});

test("not-found screen passes WCAG AA in both themes", async ({ page }) => {
  await auditRoute(page, "/#/definitely-not-a-route", "not-found");
});
