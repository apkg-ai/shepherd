/**
 * Spec-derived API client (S7). Pairs with scripts/regen-generated.sh on the
 * Rust side: committed generated code is exactly `npm run generate:api`
 * (orval + oxfmt), gated in CI by the ui-generated-drift job.
 */
import { defineConfig } from "orval";

export default defineConfig({
  // TanStack Query hooks + fetch client + MSW mock handlers.
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
        // Plain data return types — shepherdFetch resolves res.json(), not an
        // HTTP envelope. Errors travel as thrown ShepherdError instead.
        fetch: { includeHttpResponseReturnType: false },
        query: {
          // GET → useQuery (default); paginated GETs also get infinite hooks
          // keyed on the spec's `cursor` param. Non-GET verbs stay mutations.
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
