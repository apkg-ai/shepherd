# shepherd UI

Vite + React + TypeScript app. `npm run dev` proxies `/api` to the shepherd
daemon on `127.0.0.1:7437`; `npm run build` emits `dist/`, served by
`shepherd-server`. Routing is hash-based so the built app works from the
server's static fallback (and a future Tauri shell) without SPA rewrites.

Current state: the v1 step-000 scaffold — one accessible shell route that
renders the daemon's live health. Application routes return with their owning
steps in `../plan/`.

Scripts: `dev`, `build`, `generate:api` (orval + oxfmt, see below), `lint`
(oxlint), `fmt` / `fmt:check` (oxfmt), `typecheck` (tsc), `test` (vitest),
`test:unit:cov` (vitest + v8 coverage), `test:e2e` (Playwright via
`../scripts/playwright-e2e.sh`).

## Spec-derived API client

`src/api/generated/` is generated (gitignored) from `../openapi/shepherd.yaml`
by [orval](https://orval.dev) (`orval.config.ts`):

- TanStack Query hooks + fetch client (per spec tag), routed through the
  `shepherdFetch` mutator (`src/api/client.ts`) which normalizes RFC 9457
  problem+json errors into `ShepherdError` (`src/api/problem.ts`);
- MSW mock handlers (`*.msw.ts`) — the test suite's spec-shaped background
  (`src/test/msw.ts`);
- zod schemas (`generated/zod/`) — client-side form validation that cannot
  drift from the server's 422 rules.

Never edit generated files. After changing the spec, run:

```sh
npm run generate:api
```

Generated code is lint-ignored (`.oxlintrc.json`) and excluded from the
coverage gate (`vite.config.ts`); CI regenerates it before every build/test.

## Tests

Screen-level integration tests render real routes (memory router over the
shared route table in `src/router.tsx`) against MSW: generated handlers serve
spec-shaped faker noise, and each test seeds the deterministic responses it
asserts on (`server.use(...)` — handlers registered first win). Coverage
gate: ≥95% lines on `ui/src` (`scripts/coverage-report.mjs`).

Playwright specs (`e2e/`) run against the real Rust server serving `dist/`:
shell + theme behavior (`shell.spec.ts`) and an axe WCAG-AA audit of every
route in both themes (`a11y.spec.ts`).
