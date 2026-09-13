import { expect, test } from "./helpers/coverage";
import { createKnowledge, createProject, createTask } from "./helpers/api";

test.describe("project knowledge", () => {
  test("lists, filters, and searches the project's knowledge", async ({ page }) => {
    const project = await createProject(`Knowledge ${Date.now()}`);
    const task = await createTask(project.id, "Knowledge source task");
    await createKnowledge(project.id, {
      title: "House conventions",
      content: "Small commits, spec first.",
    });
    await createKnowledge(project.id, {
      title: "Graph library decision",
      content: "React Flow with dagre.",
      type: "decision",
      scope: "task",
      taskId: task.id,
    });

    await page.goto(`/#/projects/${project.id}/knowledge`);
    await expect(page.getByText("House conventions")).toBeVisible();
    await expect(page.getByText("Graph library decision")).toBeVisible();
    await expect(page.getByRole("status")).toHaveText(/2 of 2 items/);

    // Search narrows client-side across title and content.
    await page.getByLabel("Search").fill("dagre");
    await expect(page.getByRole("status")).toHaveText(/1 of 2 items/);
    await expect(page.getByText("House conventions")).toBeHidden();

    // Provenance links back to the source task.
    await page.getByRole("link", { name: "From task" }).click();
    await expect(page.getByRole("heading", { name: "Knowledge source task" })).toBeVisible();

    // Scope filter narrows server-side.
    await page.goto(`/#/projects/${project.id}/knowledge?scope=project`);
    await expect(page.getByText("House conventions")).toBeVisible();
    await expect(page.getByText("Graph library decision")).toBeHidden();
  });
});
