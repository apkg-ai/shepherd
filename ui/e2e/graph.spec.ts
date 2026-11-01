import { expect, test } from "./helpers/coverage";
import { login } from "./helpers/auth";

test.describe("dependency graph", () => {
  let projectId: string;
  let goalId: string;
  let epicIds: string[];

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

    const project = await post("/api/v1/projects", { name: "Graph Test" });
    projectId = project.id;

    const goal = await post(`/api/v1/projects/${projectId}/goals`, { title: "Graph Goal" });
    goalId = goal.id;

    const epicA = await post(`/api/v1/projects/${projectId}/epics`, {
      goal_id: goalId,
      title: "Epic Alpha",
    });
    const epicB = await post(`/api/v1/projects/${projectId}/epics`, {
      goal_id: goalId,
      title: "Epic Beta",
    });
    const epicC = await post(`/api/v1/projects/${projectId}/epics`, {
      goal_id: goalId,
      title: "Epic Gamma",
    });
    epicIds = [epicA.id, epicB.id, epicC.id];

    // Alpha -> Beta and Alpha -> Gamma
    await post(`/api/v1/projects/${projectId}/dependencies`, {
      level: "epic",
      dependent_id: epicB.id,
      prerequisite_id: epicA.id,
    });
    await post(`/api/v1/projects/${projectId}/dependencies`, {
      level: "epic",
      dependent_id: epicC.id,
      prerequisite_id: epicA.id,
    });
  });

  test("goal detail shows graph toolbar", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);

    const toolbar = page.getByRole("group", { name: "Graph controls" });
    await expect(toolbar).toBeVisible();
    await expect(page.getByRole("button", { name: "List view" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Fit to view" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("toggle between graph and list views", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);

    // Wait for graph to load — toolbar and Fit button indicate graph mode is active
    await expect(page.getByRole("button", { name: "Fit to view" })).toBeVisible();

    // Switch to list view
    await page.getByRole("button", { name: "List view" }).click();
    await expect(page.getByText("Epic Alpha")).toBeVisible();
    await expect(page.getByText("Epic Beta")).toBeVisible();
    await expect(page.getByText("Epic Gamma")).toBeVisible();

    // Switch back to graph view
    await page.getByRole("button", { name: "Graph view" }).click();
    await expect(page.getByRole("button", { name: "Fit to view" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Zoom in" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Zoom out" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("list view shows epic cards with status filter", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);
    await page.getByRole("button", { name: "List view" }).click();

    const filterGroup = page.getByRole("group", { name: "Filter by status" });
    await expect(filterGroup).toBeVisible();
    await expect(page.getByRole("button", { name: "Active" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Done" })).toBeVisible();
    await expect(page.getByRole("button", { name: "All" })).toBeVisible();

    // Active filter is pressed by default
    await expect(page.getByRole("button", { name: "Active" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // All three epics visible under Active (all are open/active by default)
    await expect(page.getByText("Epic Alpha")).toBeVisible();
    await expect(page.getByText("Epic Beta")).toBeVisible();
    await expect(page.getByText("Epic Gamma")).toBeVisible();

    // All filter shows everything
    await page.getByRole("button", { name: "All" }).click();
    await expect(page.getByText("Epic Alpha")).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("clicking node opens side panel and navigates to detail", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);

    // Click a graph node to open the side panel
    const node = page.locator(`.react-flow__node[data-id="${epicIds[0]}"]`);
    await expect(node).toBeVisible();
    await node.click();

    // Side panel shows node details
    const panel = page.getByRole("complementary", { name: "Selected item details" });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("Epic Alpha")).toBeVisible();

    // Navigate via "Open details" link
    await panel.getByRole("link", { name: "Open details" }).click();
    await expect(page).toHaveURL(new RegExp(`#/projects/${projectId}/epics/${epicIds[0]}$`));
    await expect(page.getByRole("heading", { name: "Epic Alpha" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("keyboard navigation: Enter on list view toggle works", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);

    // Focus the List view button and activate with Enter
    const listViewButton = page.getByRole("button", { name: "List view" });
    await listViewButton.focus();
    await page.keyboard.press("Enter");

    await expect(page.getByRole("group", { name: "Filter by status" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test("no console errors during graph interaction", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("Content Security Policy"))
        consoleErrors.push(msg.text());
    });

    await login(page);
    await page.goto(`/#/projects/${projectId}/goals/${goalId}`);

    // Interact with toolbar controls
    await expect(page.getByRole("group", { name: "Graph controls" })).toBeVisible();
    await page.getByRole("button", { name: "Fit to view" }).click();
    await page.getByRole("button", { name: "Zoom in" }).click();
    await page.getByRole("button", { name: "Zoom out" }).click();

    // Toggle views back and forth
    await page.getByRole("button", { name: "List view" }).click();
    await expect(page.getByText("Epic Alpha")).toBeVisible();
    await page.getByRole("button", { name: "Graph view" }).click();
    await expect(page.getByRole("button", { name: "Fit to view" })).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });
});
