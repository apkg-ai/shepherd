import { expect, test } from "./helpers/coverage";
import { login } from "./helpers/auth";

test.describe("hierarchy flow", () => {
  test("create project → goal → epic → task and navigate back", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      // CSP inline-script error is from the server's Content-Security-Policy header,
      // not from application code — filter it out.
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto("/#/");

    // Create project
    await page.getByRole("button", { name: "New project" }).click();
    await page.getByLabel("Name").fill("E2E Test Project");
    await page.getByRole("button", { name: "Create project" }).click();
    await expect(page.getByRole("heading", { name: "E2E Test Project" })).toBeVisible();

    // Create goal
    await page.getByRole("link", { name: "New goal" }).click();
    await page.getByLabel("Title").fill("E2E Goal");
    await page.getByRole("button", { name: "Create goal" }).click();
    await expect(page.getByRole("heading", { name: "E2E Goal" })).toBeVisible();

    // Create epic
    await page.getByRole("link", { name: "New epic" }).click();
    await page.getByLabel("Title").fill("E2E Epic");
    await page.getByRole("button", { name: "Create epic" }).click();
    await expect(page.getByRole("heading", { name: "E2E Epic" })).toBeVisible();

    // Create task
    await page.getByRole("link", { name: "New task" }).click();
    await expect(page.getByLabel("Title")).toBeVisible();
    await page.getByLabel("Title").fill("E2E Task");
    await page.getByRole("button", { name: "Create task" }).click();
    await expect(page.getByRole("heading", { name: "E2E Task" })).toBeVisible();

    // Verify task detail renders badges and metadata
    await expect(page.getByText("Execution", { exact: true })).toBeVisible();
    await expect(page.getByText("Open", { exact: true }).first()).toBeVisible();
    await expect(page.getByRole("region", { name: "Task metadata" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Eligibility" })).toBeVisible();

    // Action panel renders from the server's allowed_actions vocabulary
    // (REST operationIds), not a client-side fabrication.
    await expect(page.getByText("No actions available")).toBeHidden();
    await expect(page.getByRole("button", { name: "Block" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Cancel task" })).toBeVisible();

    // Navigate back via breadcrumbs
    await page.getByRole("link", { name: "E2E Epic" }).click();
    await expect(page.getByRole("heading", { name: "E2E Epic" })).toBeVisible();
    await expect(page.getByText("E2E Task")).toBeVisible();

    await page.getByRole("link", { name: "E2E Goal" }).click();
    await expect(page.getByRole("heading", { name: "E2E Goal" })).toBeVisible();

    await page.getByRole("link", { name: "E2E Test Project" }).click();
    await expect(page.getByRole("heading", { name: "E2E Test Project" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("edit goal preserves data and navigates back", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      // CSP inline-script error is from the server's Content-Security-Policy header,
      // not from application code — filter it out.
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto("/#/");

    await page.getByRole("link", { name: "E2E Test Project" }).click();
    await page.getByRole("link", { name: "E2E Goal" }).click();
    await page.getByRole("link", { name: "Edit" }).click();

    await expect(page.getByLabel("Title")).toHaveValue("E2E Goal");

    await page.getByLabel("Title").fill("Updated Goal");
    await page.getByRole("button", { name: "Save changes" }).click();
    await expect(page.getByRole("heading", { name: "Updated Goal" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("status filter toggles task visibility", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      // CSP inline-script error is from the server's Content-Security-Policy header,
      // not from application code — filter it out.
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto("/#/");

    await page.getByRole("link", { name: "E2E Test Project" }).click();
    await page.getByRole("link", { name: "Updated Goal" }).click();
    await page.getByRole("link", { name: "E2E Epic" }).click();

    // Default Active filter shows the task
    await expect(page.getByText("E2E Task")).toBeVisible();

    // Done filter — no done tasks yet
    await page.getByRole("button", { name: /Done/ }).click();
    await expect(page.getByText("No items match this filter.")).toBeVisible();

    // All filter — shows everything
    await page.getByRole("button", { name: /All/ }).click();
    await expect(page.getByText("E2E Task")).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });
});
