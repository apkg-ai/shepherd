import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";
import { server } from "./test/msw";

// Without vitest globals, RTL cannot self-register its auto-cleanup —
// register it here so the DOM never leaks between tests (#21).
afterEach(cleanup);

// Spec-derived MSW server: generated handlers as background, per-test
// overrides via server.use(...). Unhandled requests are bugs.
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
