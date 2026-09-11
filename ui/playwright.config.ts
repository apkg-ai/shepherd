import { defineConfig, devices } from "@playwright/test";

/**
 * E2E suite against a real shepherd-server serving ui/dist — booted by
 * scripts/playwright-e2e.sh (mirrors hurl-e2e.sh: temp DB, port 7543,
 * health poll, trap cleanup). No webServer block on purpose: the boot
 * script is shared between local runs and CI.
 */
export default defineConfig({
  testDir: "./e2e",
  outputDir: "./e2e-artifacts/test-results",
  fullyParallel: false,
  // One worker: specs share a server and seed real data — determinism over speed.
  workers: 1,
  forbidOnly: !!process.env["CI"],
  retries: process.env["CI"] ? 1 : 0,
  reporter: process.env["CI"] ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL: process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:7543",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
