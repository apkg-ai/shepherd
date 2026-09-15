# 000 — Reset to a green v1 scaffold

Status: not started. Requirements: OPS-01.

## Objective and prerequisites

Remove the MVP product implementation while retaining the proven repository, build, generation, test and delivery plumbing. Required completed steps: None; the repository MVP and this handbook are the starting point.

Read [execution rules](README.md) first, then:

- [current codebase audit](../02-current-codebase-audit.md)
- [backend specification](../05-backend.md)
- [frontend specification](../06-frontend.md)
- [test strategy](../14-test-strategy.md)
- [dependency choices](../dependencies.md)

Starting state: PR #56 contains the authoritative stable-v1 handbook. The tracked MVP application still compiles. User data and the user's untracked `docs/agent-quickstart.md` and `example/` paths are outside this cleanup.

## Files and boundaries

Preserve and adapt only as necessary:

- repository metadata, license, `.gitignore`, pinned toolchain files and `.nvmrc`
- root, `core/` and `ui/` package manifests and lockfiles
- `.github/workflows/`, dependency/SAST configuration and coverage reporting
- build, format, lint, test, Hurl, Playwright, smoke and generation scripts
- Rust workspace/crate boundaries and a minimal server binary/library shell
- Vite/React/TypeScript configuration, `ui/src/main.tsx`, test setup, generic accessible components, design tokens and framework-independent utilities
- the `plan/` handbook and its executable contracts/examples

Remove or replace in this step:

- MVP Rust domain, lifecycle, store, export, event and transport behavior
- all MVP SQL migrations and compiled references to the MVP schema
- MVP OpenAPI/AsyncAPI operations, generated application types and operation allowlists
- product-specific UI routes, screens, API hooks, fixtures and behavioral tests
- MVP Hurl/Playwright scenarios and tracked product documentation that claims obsolete behavior

Create a minimal health-only contract, server route, React shell, unit test, Hurl smoke and Playwright shell check so every retained quality gate still executes meaningful work. Do not retain an MVP module merely to satisfy an old test. Generic code may be copied back from Git history in its owning later step only after its assumptions are checked against the v1 specification.

## Ordered implementation

1. Record a path-level keep/remove inventory in the implementation handoff before editing. Use Git history as the recovery mechanism; do not create a legacy source directory or compatibility crate.
2. Reduce `shepherd-core` to an importable crate shell and `shepherd-server` to health, static UI serving, configuration and graceful process startup. Remove MVP migrations and ensure the scaffold never opens or mutates `~/.shepherd/shepherd.db`.
3. Reduce the browser application to its provider/theme/error boundary and one accessible scaffold route. Retain only components and utilities that have no MVP domain vocabulary or generated-model dependency.
4. Replace the application contract with a minimal health-only scaffold contract. Keep generation, lint and compilation scripts operational; remove stale generated outputs and allowlists.
5. Replace old semantic tests with explicit scaffold tests. Update CI paths or coverage inputs only where deleted targets require it; do not skip, soften or delete a quality/security job.
6. Run all checks below from a clean checkout state and complete the handoff with the exact retained paths, removed paths and any generic helper deferred for later reconsideration.

## Acceptance tests and expected results

`cargo test --workspace`, Clippy, UI lint/typecheck/unit/build, spec lint, Hurl health, Playwright shell, smoke, dependency scans and Semgrep can all run on the scaffold. The server starts from an unrelated working directory, returns a truthful health response and serves the React shell. No compiled source, route, migration, fixture or visible text contains the MVP epic-as-task, knowledge, relation-parent or old lifecycle model. No existing user database is opened, changed, copied or deleted. No CI job is disabled and no success stub exists for a future v1 operation.

For every acceptance sentence, create a named check with setup → action → expected result. Search results are supporting evidence, not a substitute for compilation and process tests. A removed test must be obsolete because its product behavior was removed; retain infrastructure and security assertions that still apply.

## Excluded work

Do not implement the stable-v1 domain, full contracts, database baseline, application routes or compatibility adapters in this step. Do not add dependencies. Do not modify or delete user data, the user's untracked files, Git history, the handbook, CI secrets or repository settings.

## Verification

```sh
# Repository root, with Node 26 active.
python3 plan/validate.py
node plan/validate-contracts.cjs
npm run lint:specs
npm run lint --prefix ui
npm run fmt:check --prefix ui
npm run typecheck --prefix ui
npm run test:unit:cov --prefix ui
npm run build --prefix ui

# core/
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# Repository root; each script must exercise the retained scaffold.
scripts/hurl-e2e.sh
scripts/playwright-e2e.sh
scripts/smoke.sh
```

Expected: zero exit status from all commands, no skipped quality category, and no product behavior beyond health/static shell. Dependency and SAST workflows must also pass on the pull request before the step is complete.

## Completion checklist

- [ ] Keep/remove inventory recorded before deletion and reconciled afterward.
- [ ] Rust and browser scaffolds compile, start and pass meaningful smoke tests.
- [ ] MVP product code, schema, routes, screens, generated types, fixtures and obsolete documentation are absent.
- [ ] CI, security, dependency, generation, coverage and test plumbing remain operational.
- [ ] Existing user data and untracked paths remain untouched.
- [ ] No future v1 behavior or compatibility layer was introduced.
- [ ] Handoff below completed; step 001 linked as the only next eligible step.

## Implementation handoff record

Branch: `000-scaffold-reset` (off `main` at `4b565e2`). Commit/PR/CI run: recorded at completion below.

### Keep/remove inventory (recorded before deletion)

Removed (tracked paths):

- `core/shepherd-core/src/{model,lifecycle,dag,lease,bundle,export,event,error,store}.rs`
- `core/shepherd-core/migrations/` (8 SQL files, 20260907–20260910)
- `core/shepherd-core/tests/{migration_tests.rs,proptest_invariants.rs}` and proptest regression files
- `core/shepherd-server/src/convert.rs`
- `openapi/shepherd-events.asyncapi.yaml`, `.spectral-asyncapi.yaml`, root `lint:events` script
- `docs/` (8 tracked MVP documents; superseded by `plan/`)
- `tests/hurl/02…09-*.hurl` (8 MVP scenario files)
- `ui/src/screens/{AppLayout.tsx,AppLayout.module.css,shell.test.tsx,error-paths.test.tsx}` and `ui/src/screens/{graph,knowledge,projects,review,tasks}/**`
- `ui/src/components/{ReasonDialog,AttemptBadge,StatusBadge,TypeBadge,IdentityChip}.{tsx,module.css}`
- `ui/src/lib/{events.ts,events.test.tsx,graphLayout.*,graphNeighbors.*,taskTree.*,download.*,links.*}`
- `ui/src/api/{invalidate.ts,paging.ts,paging.test.ts}`
- `ui/src/styles/reactflow.css`, `ui/src/test/fixtures.ts`
- `ui/e2e/{a11y,detail,graph,keyboard,knowledge,registry,review,screenshots,settings,tasks,theme,tree}.spec.ts`, `ui/e2e/helpers/api.ts`
- CI jobs `core-test-property` and `core-test-migration` (their entire subject — the domain store and MVP migrations — is deleted; suites return with their owning steps)
- Dependencies that become unused: shepherd-core `sqlx,base64,chrono,uuid,thiserror,tokio,serde,serde_json,proptest`; shepherd-server `dirs,tokio-stream` (+`chrono,uuid` if the regenerated `REQUIRED_DEPS.toml` no longer declares them); ui `@dagrejs/dagre,@xyflow/react`; root `@axe-core/playwright`; RUSTSEC-2023-0071 ignores in `osv-scanner.toml`/`.cargo/audit.toml` (sqlx leaves the tree)

Retained (adapted where noted):

- Repository metadata, `LICENSE`, `.gitignore`, `rust-toolchain.toml`, `.nvmrc`, root/`core/`/`ui/` manifests and lockfiles (trimmed/regenerated)
- `openapi/shepherd.yaml` replaced by a health-only scaffold contract (strict subset of the MVP spec: `getHealth` verbatim, shared error/header components)
- `core/Cargo.toml` workspace; `shepherd-core/src/lib.rs` reduced to the `version()` shell + its test; `shepherd-server/src/{main.rs,lib.rs,middleware.rs}` reduced to config/startup, health, spec serving, static UI, CORS and rate-limit headers; `openapi-to-rust.toml` allowlist reduced to `getHealth`; `tests/api.rs` retargeted to scaffold checks; new `tests/boot.rs` (unrelated-cwd boot + `~/.shepherd` guard)
- `.github/workflows/{quality-gates,dependency-scan,sast}.yaml` (quality-gates minus the two removed jobs and the AsyncAPI lint step; scan/SAST untouched)
- `scripts/{regen-generated.sh,coverage-report.mjs,smoke.sh,hurl-e2e.sh,playwright-e2e.sh}` (the last three lose temp-DB/`--db` plumbing; hurl runner loses the SSE section); `tests/hurl/01-health-and-spec.hurl`
- `ui/`: `index.html`, `public/favicon.svg`, `vite.config.ts`, `orval.config.ts`, `playwright.config.ts`, all tsconfigs, `.oxlintrc.json`, `.oxfmtrc.json`; `src/main.tsx`, `src/index.css`, `src/router.tsx` (rewritten to one scaffold route), `src/styles/tokens.css` (status/type/epic families pruned) + `surfaces.module.css`; components `Button, Dialog, ConfirmDialog, FormField, CopyButton, PageHeader, LoadMore, Toast, ThemeToggle, states` (+ rewritten `components.test.tsx`); `src/lib/{theme,contrast,format,forms,queryClient}` (+tests, contrast pairs pruned in lockstep); `src/api/{client,problem}` (+tests); `src/test-setup.ts` (React Flow shims removed), `src/test/{msw.ts (reduced),test-utils.tsx}`; screens `RouteError` (+test), `NotFound`, new `AppShell`/`HomeScreen` (+tests); `e2e/helpers/{coverage.ts,global-teardown.ts}` + new `shell.spec.ts`/`a11y.spec.ts`
- The entire `plan/` handbook (only this handoff section changes)

Generic helpers intentionally deferred (recover from Git history at the owning step after checking assumptions against the v1 specification): `ProblemDetailRemapLayer` (step 015), `ui/src/lib/download.ts` (step 014), `ui/src/lib/links.ts` (step 019), `ui/src/api/paging.ts` and `ui/src/api/invalidate.ts` (step 016), React Flow jsdom shims in `test-setup.ts` (step 018).

### Results (completed at end of implementation)

Additions beyond the inventory (created, not retained): `core/shepherd-server/tests/boot.rs` (unrelated-cwd boot + HOME guard), `ui/src/screens/{AppShell,HomeScreen}.tsx` (+css), `ui/src/screens/shell.test.tsx`, `ui/e2e/{shell,a11y}.spec.ts`. The inventory above was reconciled against `git status` after implementation — no unplanned path was touched.

Test commands and results (from the reset tree, Node via nvm, Rust 1.98.1):

- `python3 plan/validate.py` — PASS (29 step DAG; 70 operations).
- `node plan/validate-contracts.cjs` — PASS (108 examples).
- `npm run lint:specs` — PASS (spectral, zero errors on the health-only contract).
- `npm run lint --prefix ui` / `fmt:check` / `typecheck` — PASS (3 pre-existing fast-refresh warnings, zero errors).
- `npm run test:unit:cov --prefix ui` — 127/127 passed, 97.1% lines (gate ≥95%).
- `npm run build --prefix ui` — PASS.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` — PASS.
- `cargo test --workspace` (incl. `--doc`) — 10/10 passed: 1 core unit, 8 api integration/contract, 1 boot guard.
- `scripts/hurl-e2e.sh` — 1/1 hurl file passed.
- `scripts/playwright-e2e.sh` — 7/7 specs passed (shell, theme, axe WCAG-AA both themes).
- `scripts/smoke.sh` — OK (health, spec, UI index).
- `node scripts/coverage-report.mjs --check` with all four lcov suites — thresholds met: core unit 100%, integration 100%, total 100%; ui unit 97.1%, e2e 69.7%, total 94.5%.
- Negative sweep: `grep -riE 'epic|knowledge|lease|claim|proposed|in_review|depends_on|decomposition|sqlite|sqlx'` over compiled sources, tests, specs and scripts — zero MVP-domain hits.

CI run: PR [#81](https://github.com/apkg-ai/shepherd/pull/81) — all checks green: [Quality Gates](https://github.com/apkg-ai/shepherd/actions/runs/34939434689) (16 jobs incl. coverage gate), [Dependency Scan](https://github.com/apkg-ai/shepherd/actions/runs/34939434603), [SAST](https://github.com/apkg-ai/shepherd/actions/runs/34939434666). One follow-up commit on the branch bumps transitive `rustls` 0.23.44→0.23.45 in `core/Cargo.lock` for RUSTSEC-2026-0285 (advisory published 2026-09-14, unrelated to the reset).

User data and untracked paths untouched: `~/.shepherd/shepherd.db` (+wal/shm) last modified 2026-09-13, before this implementation; no test or script resolves that path anymore, and `tests/boot.rs` asserts the server never creates `~/.shepherd`. `docs/agent-quickstart.md` and `example/` do not exist in this working tree; nothing was created there.

Next eligible step: 001.
