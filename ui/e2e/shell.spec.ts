import { expect, test } from "./helpers/coverage";

/**
 * Scaffold shell checks (v1 step 000) against the real Rust server serving
 * ui/dist — the browser → fetch → axum → shepherd-core path end to end.
 */

test.describe("scaffold shell", () => {
  test("home renders live health from the daemon", async ({ page }) => {
    await page.goto("/#/");

    await expect(page.getByRole("heading", { level: 1, name: "Shepherd" })).toBeVisible();
    // Health facts come from GET /health — not fixtures.
    await expect(page.getByText("pass", { exact: true })).toBeVisible();
    await expect(page.getByText(/^\d+\.\d+\.\d+/)).toBeVisible();
    await expect(page.getByText("shepherd local daemon")).toBeVisible();
  });

  test("skip link jumps to the main region", async ({ page }) => {
    await page.goto("/#/");
    await page.keyboard.press("Tab");
    const skipLink = page.getByRole("link", { name: "Skip to content" });
    await expect(skipLink).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/#main$/);
  });

  test("unknown routes show NotFound with a way back home", async ({ page }) => {
    await page.goto("/#/definitely-not-a-route");
    await expect(page.getByText("Page not found")).toBeVisible();
    await page.getByRole("link", { name: "Back home" }).click();
    await expect(page.getByText("pass", { exact: true })).toBeVisible();
  });
});

test.describe("dark theme", () => {
  test("manual toggle applies and survives reload", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

    await page.getByRole("button", { name: "Dark" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("button", { name: "Dark" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await page.reload();
    // The pre-paint script restores the stored preference before React loads.
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await page.getByRole("button", { name: "Light" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("system preference resolves via prefers-color-scheme", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto("/#/");
    await page.getByRole("button", { name: "System" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await page.emulateMedia({ colorScheme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });
});
