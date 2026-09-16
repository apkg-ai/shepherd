/* oxlint-disable react-hooks/rules-of-hooks -- Playwright fixtures name their
   continuation `use`; nothing React-y happens in this file. */
/**
 * Coverage-aware `test` (gated on E2E_COVERAGE): pages feed monocart →
 * coverage/e2e.lcov, union-merged with the vitest lcov by CI.
 */
import { existsSync } from "node:fs";
import { test as base, expect } from "@playwright/test";
import MCR, { type CoverageReportOptions } from "monocart-coverage-reports";

const enabled = !!process.env["E2E_COVERAGE"];

/** "ui/src/..." or "src/..." (sourcemap-root dependent) → "src/...". */
function normalize(sourcePath: string): string | null {
  const index = sourcePath.indexOf("src/");
  return index >= 0 ? sourcePath.slice(index) : null;
}

export const coverageOptions: CoverageReportOptions = {
  name: "shepherd UI e2e",
  outputDir: "./coverage/e2e-raw",
  reports: [["lcovonly", { file: "../e2e.lcov" }]],
  entryFilter: (entry) => entry.url.includes("/assets/"),
  // Vendored sourcemaps would collide with our src/ — keep only real ui/src files.
  sourceFilter: (sourcePath) => {
    const local = normalize(sourcePath);
    return local !== null && !local.includes("api/generated") && existsSync(local);
  },
  sourcePath: (filePath) => normalize(filePath) ?? filePath,
};

const mcr = MCR(coverageOptions);

export const test = base.extend({
  page: async ({ page }, use) => {
    if (!enabled) {
      await use(page);
      return;
    }
    await page.coverage.startJSCoverage({ resetOnNavigation: false });
    await use(page);
    const coverage = await page.coverage.stopJSCoverage();
    await mcr.add(coverage);
  },
});

export { expect };
