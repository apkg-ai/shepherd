import MCR from "monocart-coverage-reports";
import { coverageOptions } from "./coverage";

/** Generates coverage/e2e.lcov after the run when E2E_COVERAGE is set. */
export default async function globalTeardown(): Promise<void> {
  if (!process.env["E2E_COVERAGE"]) return;
  const mcr = MCR(coverageOptions);
  await mcr.generate();
}
