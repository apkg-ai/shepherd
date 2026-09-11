import { expect, test } from "@playwright/test";
import { createProject, createTask } from "./helpers/api";

test.describe("task creation and list", () => {
  test("creates an approved task with metadata and lands on its detail", async ({ page }) => {
    const project = await createProject(`Task create ${Date.now()}`);
    await page.goto(`/#/projects/${project.id}/tasks/new`);

    await page.getByLabel("Title").fill("Build the thing");
    await page.getByLabel("Description").fill("End to end");
    await page.getByLabel("Type").selectOption("refactor");
    await page.getByLabel("Metadata").fill('{"branch": "e2e"}');
    await page.getByRole("button", { name: "Create task" }).click();

    await expect(page.getByRole("heading", { name: "Build the thing" })).toBeVisible();
    await expect(page.getByText('"branch": "e2e"')).toBeVisible();
    // Human-authored default is approved; with no dependencies the server
    // auto-transitions it to ready immediately.
    await expect(page.getByText("Ready", { exact: true })).toBeVisible();
  });

  test("creates a proposal via the initial-status radio", async ({ page }) => {
    const project = await createProject(`Proposal create ${Date.now()}`);
    await page.goto(`/#/projects/${project.id}/tasks/new`);

    await page.getByLabel("Title").fill("An agent-ish idea");
    await page.getByRole("radio", { name: /Proposed — needs human approval/ }).click();
    await page.getByRole("button", { name: "Create task" }).click();

    await expect(page.getByRole("heading", { name: "An agent-ish idea" })).toBeVisible();
    await expect(page.getByText("Proposed", { exact: true })).toBeVisible();
  });

  test("filters via search params and deep links work", async ({ page }) => {
    const project = await createProject(`Filters ${Date.now()}`);
    await createTask(project.id, "Code work", { type: "code" });
    await createTask(project.id, "Research work", { type: "research" });

    // Hash deep link straight into a filtered list.
    await page.goto(`/#/projects/${project.id}?type=research`);
    await expect(page.getByRole("link", { name: "Research work" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Code work" })).toBeHidden();

    // Clearing the filter brings everything back.
    await page.getByLabel("Type").selectOption("");
    await expect(page.getByRole("link", { name: "Code work" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Research work" })).toBeVisible();
  });
});
