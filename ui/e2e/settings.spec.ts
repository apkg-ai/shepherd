import { expect, test } from "@playwright/test";
import { createProject } from "./helpers/api";

test.describe("project settings", () => {
  test("renames the project and toggles the review gate", async ({ page }) => {
    const project = await createProject(`Settings ${Date.now()}`);
    await page.goto(`/#/projects/${project.id}/settings`);

    const renamed = `Renamed ${Date.now()}`;
    await page.getByLabel("Name").fill(renamed);
    await page.getByRole("checkbox").uncheck();
    await page.getByRole("button", { name: "Save settings" }).click();

    await expect(page.getByText("Project settings saved.")).toBeVisible();
    // Sidebar picks up the new name via invalidation.
    await expect(
      page.getByRole("navigation", { name: "Project", exact: true }).getByText(renamed),
    ).toBeVisible();
  });

  test("exports the project as a JSON download", async ({ page }) => {
    const project = await createProject(`Export me ${Date.now()}`);
    await page.goto(`/#/projects/${project.id}/settings`);

    const downloadPromise = page.waitForEvent("download");
    await page.getByRole("button", { name: "Export project" }).click();
    const download = await downloadPromise;
    expect(download.suggestedFilename()).toMatch(/^export-me-\d+-export\.json$/);
  });

  test("deletes the project after confirmation", async ({ page }) => {
    const project = await createProject(`Doomed ${Date.now()}`);
    await page.goto(`/#/projects/${project.id}/settings`);

    await page.getByRole("button", { name: "Delete project" }).click();
    await page
      .getByRole("dialog", { name: "Delete project" })
      .getByRole("button", { name: "Delete project" })
      .click();

    await expect(page.getByText("Project deleted.")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible();
    await expect(page.getByRole("main").getByText(project.name)).toHaveCount(0);
  });
});
