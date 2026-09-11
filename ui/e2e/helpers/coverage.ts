/* oxlint-disable react-hooks/rules-of-hooks -- Playwright fixtures name their
   continuation `use`; nothing React-y happens in this file. */
/**
 * Coverage-aware `test`: with E2E_COVERAGE set (CI does), every page collects
 * Chromium V8 coverage and feeds it to monocart, which unpacks sourcemaps and
 * emits coverage/e2e.lcov with SF paths relative to ui/ (src/...) so it
 * union-merges with the vitest lcov in scripts/coverage-report.mjs.
 * Specs import { test, expect } from here instead of @playwright/test.
 */
import { test as base, expect } from "@playwright/test";
import MCR, { type CoverageReportOptions } from "monocart-coverage-reports";

const enabled = !!process.env["E2E_COVERAGE"];

export const coverageOptions: CoverageReportOptions = {
  name: "shepherd UI e2e",
  outputDir: "./coverage/e2e-raw",
  reports: [["lcovonly", { file: "../e2e.lcov" }]],
  entryFilter: (entry) => entry.url.includes("/assets/"),
  sourceFilter: (sourcePath) =>
    sourcePath.includes("src/") && !sourcePath.includes("api/generated"),
  sourcePath: (filePath) => {
    // monocart unpacks to paths like "ui/src/..." or "src/..." depending on
    // the sourcemap root — normalize everything onto "src/...".
    const index = filePath.indexOf("src/");
    return index >= 0 ? filePath.slice(index) : filePath;
  },
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
