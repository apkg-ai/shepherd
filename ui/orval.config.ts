import { defineConfig } from "orval";

export default defineConfig({
  shepherd: {
    input: { target: "../openapi/shepherd.yaml" },
    output: {
      mode: "tags-split",
      target: "src/api/generated",
      schemas: "src/api/generated/model",
      client: "react-query",
      httpClient: "fetch",
      clean: true,
      baseUrl: "", // relative /api/v1/… → Vite proxy in dev, same-origin in prod
      mock: {
        // keep handler matchers relative (same /api/v1/… the app requests)
        generators: [{ type: "msw", delay: false, baseUrl: "" }],
      },
      override: {
        mutator: { path: "src/api/client.ts", name: "shepherdFetch" },
        enumGenerationType: "const", // tsconfig erasableSyntaxOnly forbids TS enums
        // shepherdFetch resolves res.json() and throws ShepherdError — no HTTP envelope.
        fetch: { includeHttpResponseReturnType: false },
        query: {
          // Paginated GETs get infinite hooks keyed on the spec's `cursor` param.
          useInfinite: true,
          useInfiniteQueryParam: "cursor",
        },
      },
    },
  },
  // Zod schemas for client-side form validation, derived from the same spec.
  shepherdZod: {
    input: { target: "../openapi/shepherd.yaml" },
    output: {
      mode: "tags-split",
      target: "src/api/generated/zod",
      client: "zod",
      fileExtension: ".zod.ts",
      clean: true,
    },
  },
});
