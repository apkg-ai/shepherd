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

Fill when implementing, not during planning: commit/branch; exact retained and removed paths; test commands and results; CI run; any generic helper intentionally deferred; confirmation that user data/untracked paths were untouched; next eligible step 001. If any retained quality gate cannot pass, leave this step incomplete and explain the concrete blocker.
