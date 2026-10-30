import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";
import { server } from "./test/msw";
import "./i18n";

// Cold module transforms under parallel workers can exceed the 1s default;
// 5s keeps first-test findBy/waitFor waits stable on loaded runners.
configure({ asyncUtilTimeout: 5000 });

afterEach(cleanup);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
