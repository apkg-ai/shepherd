# shepherd UI

Vite + React + TypeScript app. `node --run dev` proxies `/api` to the shepherd
daemon on `127.0.0.1:7437`; `node --run build` emits `dist/`, served by
`shepherd-server`. Routing is hash-based so the built app works from the
server's static fallback without SPA rewrites.

Current state: the v1 step-000 scaffold — one accessible shell route that
renders the daemon's live health. Application routes return with their owning
steps in `../plan/`.

## Spec-derived API client

`src/api/generated/` is generated (gitignored) from `../openapi/shepherd.yaml`
by [orval](https://orval.dev) (`orval.config.ts`): TanStack Query hooks + fetch
client through the `shepherdFetch` mutator (`src/api/client.ts`, which
normalizes problem+json errors into `ShepherdError`), MSW mock handlers for
tests, and zod schemas for form validation.

Never edit generated files — after changing the spec, run
`node --run generate:api`. CI regenerates before every build/test; generated
code is lint-ignored (`.oxlintrc.json`) and excluded from the coverage gate.

## Tests

Screen-level integration tests render real routes against MSW: generated
handlers serve spec-shaped background noise, and each test seeds the
responses it asserts on (`server.use(...)` — handlers registered first win).
Coverage gate: ≥95% lines on `ui/src` (`scripts/coverage-report.mjs`).

Playwright (`e2e/`) runs against the real Rust server serving `dist/`: shell
+ theme behavior, and an axe WCAG-AA audit of every route in both themes.
