import { expect, test } from "./helpers/coverage";
import { createProject, seedInReviewTask } from "./helpers/api";

/** Keyboard usability in a real browser: modal focus trap and ARIA tabs. */

test.describe("keyboard navigation", () => {
  test("dialog traps focus and restores it to the opener", async ({ page }) => {
    await page.goto("/#/");
    const opener = page.getByRole("button", { name: "Register project" });
    await opener.click();
    const dialog = page.getByRole("dialog", { name: "Register project" });
    await expect(dialog).toBeFocused();

    // Tab enough times to lap every focusable — focus must stay inside.
    for (let i = 0; i < 12; i += 1) {
      await page.keyboard.press("Tab");
      const inside = await dialog.evaluate((el) => el.contains(document.activeElement));
      expect(inside, `focus escaped the dialog after ${i + 1} Tabs`).toBe(true);
    }

    // Shift+Tab wraps backwards without escaping either.
    await page.keyboard.press("Shift+Tab");
    expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);

    // Escape closes and gives focus back to the button that opened it.
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(opener).toBeFocused();
  });

  test("review tabs support arrow-key navigation", async ({ page }) => {
    const project = await createProject(`Kbd tabs ${Date.now()}`);
    await seedInReviewTask(project.id, "Kbd review item");
    await page.goto(`/#/projects/${project.id}/review`);

    const inReview = page.getByRole("tab", { name: /^In review/ });
    const proposals = page.getByRole("tab", { name: /^Proposals/ });
    await inReview.click();

    await page.keyboard.press("ArrowRight");
    await expect(proposals).toBeFocused();
    await expect(proposals).toHaveAttribute("aria-selected", "true");
    await expect(page.getByText("No proposals waiting")).toBeVisible();

    await page.keyboard.press("ArrowLeft");
    await expect(inReview).toBeFocused();
    await expect(page.getByRole("link", { name: "Kbd review item" })).toBeVisible();

    await page.keyboard.press("End");
    await expect(proposals).toBeFocused();
    await page.keyboard.press("Home");
    await expect(inReview).toBeFocused();
  });
});
