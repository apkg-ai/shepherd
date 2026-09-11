import { expect, test } from "./helpers/coverage";
import { createProject, createTask, reportSession, seedInReviewTask } from "./helpers/api";

test.describe("task detail", () => {
  test("shows sessions timeline with failures and hedged attempt count", async ({ page }) => {
    const project = await createProject(`Detail sessions ${Date.now()}`);
    const task = await createTask(project.id, "Bumpy ride");
    await reportSession(project.id, task.id, "failed", {
      summary: "First try",
      failureReason: "Tests exploded",
    });
    await reportSession(project.id, task.id, "succeeded", { summary: "Second try" });

    await page.goto(`/#/projects/${project.id}/tasks/${task.id}`);
    await expect(page.getByText("Tests exploded")).toBeVisible();
    await expect(page.getByText("✓ Succeeded")).toBeVisible();
    await expect(page.getByText("✗ Failed")).toBeVisible();
    await expect(page.getByText("2 attempts, 1 failed")).toBeVisible();
    // The worker identity is on the timeline entries.
    await expect(page.getByText("E2E agent").first()).toBeVisible();
  });

  test("adds and removes a depends_on relation through the dialog", async ({ page }) => {
    const project = await createProject(`Detail relations ${Date.now()}`);
    const main = await createTask(project.id, "Relation main");
    await createTask(project.id, "Relation target");

    await page.goto(`/#/projects/${project.id}/tasks/${main.id}`);
    await page.getByRole("button", { name: "Add relation" }).click();
    const dialog = page.getByRole("dialog", { name: "Add relation" });
    await dialog.getByLabel("Kind").selectOption("depends_on");
    await dialog.getByLabel("Task").selectOption({ label: "Relation target" });
    await dialog.getByRole("button", { name: "Add relation" }).click();

    await expect(page.getByText("Relation added.")).toBeVisible();
    await expect(page.getByText("Depends on")).toBeVisible();
    await expect(page.getByRole("link", { name: "Relation target" })).toBeVisible();

    await page.getByRole("button", { name: "Remove relation" }).click();
    await page
      .getByRole("dialog", { name: "Remove relation" })
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.getByText("Relation removed.")).toBeVisible();
    await expect(page.getByText("No relations yet.")).toBeVisible();
  });

  test("blocks and unblocks with a reason", async ({ page }) => {
    const project = await createProject(`Detail block ${Date.now()}`);
    const task = await createTask(project.id, "Blockable work");

    await page.goto(`/#/projects/${project.id}/tasks/${task.id}`);
    await page.getByRole("button", { name: "Block", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Block task" });
    await dialog.getByRole("textbox").fill("Waiting on credentials");
    await dialog.getByRole("button", { name: "Block" }).click();

    await expect(page.getByText("Task blocked.")).toBeVisible();
    await expect(page.getByText("Blocked", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Unblock" }).click();
    await expect(page.getByText("Task unblocked.")).toBeVisible();
  });

  test("approves work in review from the detail screen", async ({ page }) => {
    const project = await createProject(`Detail approve ${Date.now()}`);
    const task = await seedInReviewTask(project.id, "Reviewable work");

    await page.goto(`/#/projects/${project.id}/tasks/${task.id}`);
    await expect(page.getByText("⚑ In review")).toBeVisible();
    await page.getByRole("button", { name: "Approve" }).click();
    await expect(page.getByText("Work approved — task is done.")).toBeVisible();
    await expect(page.getByText("Done", { exact: true })).toBeVisible();
  });
});
