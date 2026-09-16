import { defineConfig, devices } from "@playwright/test";

// No webServer block on purpose: scripts/playwright-e2e.sh boots the server
// and is shared between local runs and CI.
export default defineConfig({
  testDir: "./e2e",
  globalTeardown: "./e2e/helpers/global-teardown",
  outputDir: "./e2e-artifacts/test-results",
  fullyParallel: false,
  // One worker: specs share a single server — determinism over speed.
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
