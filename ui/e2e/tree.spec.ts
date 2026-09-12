import { expect, test } from "./helpers/coverage";
import { createProject, createRelation, createTask } from "./helpers/api";

test.describe("task hierarchy", () => {
  test("nests subtasks with counts, collapse, and dependency chips", async ({ page }) => {
    const project = await createProject(`Tree ${Date.now()}`);
    const parent = await createTask(project.id, "Epic parent");
    const child = await createTask(project.id, "Nested child");
    const dep = await createTask(project.id, "Blocking dependency");
    await createRelation(project.id, parent.id, "decomposition", child.id);
    await createRelation(project.id, child.id, "depends_on", dep.id);

    await page.goto(`/#/projects/${project.id}/tasks`);
    await expect(page.getByRole("link", { name: "Epic parent" })).toBeVisible();
    await expect(page.getByText("1 subtask", { exact: true })).toBeVisible();

    // The child waits on its (not-done) dependency.
    await expect(page.getByText("waits on 1")).toBeVisible();

    // Collapse hides the child; expand restores it.
    await page.getByRole("button", { name: "Collapse Epic parent" }).click();
    await expect(page.getByRole("link", { name: "Nested child" })).toBeHidden();
    await page.getByRole("button", { name: "Expand Epic parent" }).click();
    await expect(page.getByRole("link", { name: "Nested child" })).toBeVisible();
  });

  test("filtered views flatten with parent breadcrumbs", async ({ page }) => {
    const project = await createProject(`Tree filtered ${Date.now()}`);
    const parent = await createTask(project.id, "Filter parent");
    const child = await createTask(project.id, "Filter child", { type: "research" });
    await createRelation(project.id, parent.id, "decomposition", child.id);

    await page.goto(`/#/projects/${project.id}/tasks?type=research`);
    await expect(page.getByRole("link", { name: "Filter child" })).toBeVisible();
    // No tree affordances when filtered; breadcrumb carries the context.
    await expect(page.getByRole("button", { name: /Collapse|Expand/ })).toHaveCount(0);
    await expect(page.getByText("↳ Filter parent")).toBeVisible();
  });
});
