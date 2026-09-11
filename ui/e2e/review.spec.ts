import { expect, test } from "@playwright/test";
import { createProject, createTask, seedInReviewTask } from "./helpers/api";

test.describe("review queue", () => {
  test("approves waiting work from the in-review tab", async ({ page }) => {
    const project = await createProject(`Review approve ${Date.now()}`);
    await seedInReviewTask(project.id, "Ship it");

    await page.goto(`/#/projects/${project.id}/review`);
    await expect(page.getByRole("link", { name: "Ship it" })).toBeVisible();
    await page.getByRole("button", { name: "Approve" }).click();

    await expect(page.getByText(/Approved “Ship it” — done\./)).toBeVisible();
    await expect(page.getByText("Nothing waiting on you")).toBeVisible();
  });

  test("rejects with a required reason, returning the task to ready", async ({ page }) => {
    const project = await createProject(`Review reject ${Date.now()}`);
    await seedInReviewTask(project.id, "Not quite");

    await page.goto(`/#/projects/${project.id}/review`);
    await page.getByRole("button", { name: "Reject" }).click();
    const dialog = page.getByRole("dialog");

    // Reason required.
    await dialog.getByRole("button", { name: "Reject" }).click();
    await expect(dialog.getByRole("textbox")).toHaveAttribute("aria-invalid", "true");

    await dialog.getByRole("textbox").fill("Pagination is broken");
    await dialog.getByRole("button", { name: "Reject" }).click();
    await expect(page.getByText(/Rejected “Not quite” — back to ready\./)).toBeVisible();
  });

  test("handles proposals: approve one, cancel another", async ({ page }) => {
    const project = await createProject(`Review proposals ${Date.now()}`);
    await createTask(project.id, "Good idea", { status: "proposed" });
    await createTask(project.id, "Bad idea", { status: "proposed" });

    await page.goto(`/#/projects/${project.id}/review?tab=proposals`);
    const goodRow = page.getByRole("listitem").filter({ hasText: "Good idea" });
    await goodRow.getByRole("button", { name: "Approve" }).click();
    await expect(page.getByText(/Approved proposal “Good idea”\./)).toBeVisible();

    const badRow = page.getByRole("listitem").filter({ hasText: "Bad idea" });
    await badRow.getByRole("button", { name: "Cancel task" }).click();
    await page
      .getByRole("dialog", { name: "Cancel proposal" })
      .getByRole("button", { name: "Cancel task" })
      .click();
    await expect(page.getByText(/Cancelled “Bad idea”\./)).toBeVisible();
    await expect(page.getByText("No proposals waiting")).toBeVisible();
  });

  test("review badge in the sidebar tracks the queue", async ({ page }) => {
    const project = await createProject(`Review badge ${Date.now()}`);
    await seedInReviewTask(project.id, "Badge fodder");

    await page.goto(`/#/projects/${project.id}`);
    const reviewLink = page
      .getByRole("navigation", { name: "Project", exact: true })
      .getByRole("link", { name: /^Review/ });
    await expect(reviewLink.getByText("1", { exact: true })).toBeVisible();
  });
});
