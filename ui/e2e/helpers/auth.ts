import type { Page } from "@playwright/test";

/**
 * Create a browser session by posting the owner token from the
 * SHEPHERD_OWNER_TOKEN env var. The resulting cookie is set on the page.
 */
export async function login(page: Page) {
  const token = process.env["SHEPHERD_OWNER_TOKEN"];
  if (!token) throw new Error("SHEPHERD_OWNER_TOKEN not set — run via scripts/playwright-e2e.sh");

  const baseURL = page.context().pages()[0]?.url()
    ? new URL(page.url()).origin
    : (process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:7543");

  const response = await page.request.post(`${baseURL}/api/v1/session`, {
    data: { owner_token: token },
  });

  if (!response.ok()) {
    throw new Error(`Login failed: ${response.status()} ${await response.text()}`);
  }
}
