import { expect, test } from "./helpers/coverage";
import { createProject, createTask, exportProject } from "./helpers/api";

test.describe("project registry", () => {
  test("registers a project through the dialog and lands on it", async ({ page }) => {
    const name = `Registered ${Date.now()}`;
    await page.goto("/#/");
    await page.getByRole("button", { name: "Register project" }).click();

    const dialog = page.getByRole("dialog", { name: "Register project" });
    await dialog.getByLabel("Name").fill(name);
    await dialog.getByLabel("Description").fill("Created by Playwright");
    await dialog.getByRole("button", { name: "Register" }).click();

    // Lands on the project's (empty) task list with the sidebar nav active.
    await expect(page.getByText("No tasks yet")).toBeVisible();
    await expect(
      page.getByRole("navigation", { name: "Project", exact: true }).getByText(name),
    ).toBeVisible();
  });

  test("lists existing projects with their review gate", async ({ page }) => {
    const name = `Listed ${Date.now()}`;
    await createProject(name, { reviewGate: false, description: "gate off" });
    await page.goto("/#/");
    const main = page.getByRole("main");
    const row = main.getByRole("row").filter({ hasText: name });
    await expect(row).toBeVisible();
    await expect(row.getByText("Off", { exact: true })).toBeVisible();
  });

  test("imports an export document as a new project", async ({ page }) => {
    // Round-trip a real export — the import endpoint validates the full
    // ExportDocument shape, so hand-rolled fixtures 422.
    const source = await createProject(`Export source ${Date.now()}`);
    await createTask(source.id, "Travelling task");
    const exported = await exportProject(source.id);
    await page.goto("/#/");
    await page.getByRole("button", { name: "Import" }).click();
    const dialog = page.getByRole("dialog", { name: "Import project" });

    const chooserPromise = page.waitForEvent("filechooser");
    await dialog.getByLabel("Export file").click();
    const chooser = await chooserPromise;
    await chooser.setFiles({
      name: "export.json",
      mimeType: "application/json",
      buffer: Buffer.from(JSON.stringify(exported)),
    });
    await dialog.getByRole("button", { name: "Import" }).click();

    await expect(
      page.getByText(/Imported 1 tasks, 0 relations, 0 sessions, 0 knowledge items\./),
    ).toBeVisible();
    // Import lands on the graph — the task shows as a node (title button).
    await expect(page.getByRole("button", { name: "Travelling task" })).toBeVisible();
  });
});
