import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Without vitest globals, RTL cannot self-register its auto-cleanup —
// register it here so the DOM never leaks between tests (#21).
afterEach(cleanup);
