# 001 — Contract baseline and generation scaffolding

Status: complete (PR #82). Requirements: API-01.

## Objective and prerequisites

Deliver the stable-v1 contract baseline and generation scaffolding. Required completed steps: [000](000-scaffold-reset.md).

Read [execution rules](README.md) first, then:

- [07-rest-contract.md](../07-rest-contract.md)
- [contracts/README.md](../contracts/README.md)
- [dependencies.md](../dependencies.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `scripts/check-v1-contracts.sh`
- `core/shepherd-server/openapi-to-rust.toml (read-only until 015)`
- `ui/orval.config.ts (read-only until 016)`

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Copy plan contracts to a temporary fixture; run Rust types/server and Orval generation. Add a contract smoke check that parses every schema and validates operation examples. Freeze operation IDs and generate the Rust-client wire-model module from the same source. Do not promote the new spec to openapi/ yet.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Every referenced schema resolves; every operation example validates; generated Rust models compile in a temporary crate and Orval output typechecks. The minimal health server and UI scaffold remain green and do not expose the future catalog.

For each sentence above create a named regression test with setup → action → expected status/error → persisted state checks. Mutation failures must leave resource revision/history unchanged except an independently committed prior command. Use file-backed SQLite and independent pools for concurrency, controllable clock for TTL; do not test only a mocked helper that mirrors implementation. For UI, cover loading/empty/error plus keyboard interaction for new controls, using MSW for component tests and real server for critical E2E.

## Excluded work

Do not implement subsequent steps or change confirmed product decisions. No external-agent spawning, recursive hierarchy, cross-goal dependency, server-wide auth bypass or MVP data converter. Only add dependencies pinned in [dependency choices](../dependencies.md). Never reset or delete the user's existing database to make tests pass.

## Verification

```sh
# Repository root.
python3 plan/validate.py
node plan/validate-contracts.cjs
python3 plan/check-generation.py
```

If Node from `.nvmrc` is not active in the shell, pass its binary directory explicitly with `python3 plan/check-generation.py --node-dir <node-bin-directory>`. This single check must generate and compile temporary Rust models/server code and generate/typecheck the Orval React Query and Zod output.

Run Rust commands from core/, not the repository root. Keep the minimal scaffold contract until step 015 wires the complete v1 REST surface; never expose successful placeholder operations. Run scripts/regen-generated.sh only when the owning contract step requires generated application files, knowing it formats Rust. For UI generation run npm run generate:api --prefix ui before typecheck when the contract changed. Hurl/Playwright need a fresh isolated database and their installed tools; use existing scripts rather than a personal running daemon.

Expected: zero exit status, all named acceptance cases pass, no changes outside this step's scope. These are future implementation checks, not claims that tests ran during document creation.

## Completion checklist

- [x] Referenced requirements and every acceptance sentence implemented.
- [x] Schemas, permissions, transitions and clients remain consistent.
- [x] Success and rejection behavior verified at the public boundary.
- [x] Required commands pass; material environmental limitation recorded accurately.
- [x] Temporary code and next-step dependencies documented.
- [x] Handoff below completed; next eligible step linked.

## Implementation handoff record

Branch: `001-contract-baseline` (off `main` at `fb0ec7e`). PR [#82](https://github.com/apkg-ai/shepherd/pull/82) — all checks green at `80cc3ed`: [Quality Gates](https://github.com/apkg-ai/shepherd/actions/runs/35381243756) (incl. the new Contracts / v1 Baseline job: 3m43s first cold run, ~1m30s warm), [Dependency Scan](https://github.com/apkg-ai/shepherd/actions/runs/35381243724), [SAST](https://github.com/apkg-ai/shepherd/actions/runs/35381243745). Later commits on the branch are documentation-only.

### Files changed

- `scripts/check-v1-contracts.sh` (new) — the orchestrating gate: `validate-plan`, `validate-examples`, `lint-openapi`/`lint-asyncapi` (spectral with the plan rulesets, `--fail-severity=warn`), `contract-smoke`, `operation-freeze`, four negative checks against mutated temp copies of `plan/`, `client-module` (direct-mode Rust client + wire models generated from `plan/contracts/openapi.yaml` and `cargo check`ed in a temp crate), `warm-generation-deps` (fetch-only mirror of check-generation's server `REQUIRED_DEPS` so its `cargo check --offline` passes on cold caches), then `plan/check-generation.py`. `--node-dir` is passed through.
- `scripts/v1-operation-ids.txt` (new) — frozen sorted list of the 70 operation IDs, deliberately outside `plan/` so a plan-side contract edit that keeps internal parity still trips the gate.
- `scripts/check-operation-freeze.ts` (new) — diffs spec operation IDs against the freeze list; `--spec` for mutated copies, `--print` for regeneration at a step that owns a contract change.
- `scripts/check-contract-smoke.ts` (new) — compiles all 69 `components.schemas` with Ajv 2020 (same formats as `plan/validate-contracts.cjs`, `date-time` from `ajv-formats` full formats, ajv resolved from the root hoisted install) and validates every inline operation example across all declared media types, iterating only HTTP method keys within path items; requires ≥1 validated example per operation except SSE `getEvents`.
- `scripts/contract-fixtures.ts` (new) — the four known mutations for the negative checks plus the warmup server-config TOML writer (mirror of the config inside `plan/check-generation.py`; update both together on a generator bump).
- `core/shepherd-server/tests/api.rs` — new `contract::catalog_operations_are_not_exposed_by_the_scaffold`: reads `plan/contracts/operations.json` via `include_str!`, asserts the catalog is 70 operations, substitutes path params with a dummy UUID and proves `getHealth` → 200 while every other operation is unrouted and never succeeds (GET → 404 from the static fallback; other methods → 405 because the fallback serves only GET/HEAD).
- `.github/workflows/quality-gates.yaml` — new job `contracts-v1` ("Contracts / v1 Baseline") running `scripts/check-v1-contracts.sh` with the existing pinned actions, `cache-all-crates: true` (temp crates live outside the workspace) and root+ui `npm ci` behind `persist-credentials: false` on checkout (PR lifecycle code must not inherit the token); nothing in CI previously exercised `plan/`.
- `tests/hurl/02-scaffold-boundary.hurl` (new, review follow-up) — the same non-exposure negatives over live HTTP (404 for catalog GETs, 405 for non-GET).
- Review follow-ups in the gate and smoke check: the gate verifies the installed `openapi-to-rust` matches the pin parsed from the composite action and pre-fetches the types-only models `REQUIRED_DEPS` for cold caches; the smoke check rejects a media object carrying an example without a schema. One Semgrep regression (`no-replaceall`) was found in CI and fixed with the repo's `replace(/…/g, …)` idiom.
- This handoff section.

`openapi/shepherd.yaml`, `core/shepherd-server/openapi-to-rust.toml` and `ui/orval.config.ts` are untouched; no dependency manifest changed (ajv is used from the existing hoisted install exactly as `plan/validate-contracts.cjs` does).

### Acceptance mapping

- Every referenced schema resolves — `validate-plan`, `contract-smoke`, `negative-broken-ref` (mutated `$ref` → copied `validate.py` fails on `__MissingSchema`).
- Every operation example validates — `validate-examples` (108 catalog examples), `contract-smoke` (107 inline spec examples), `negative-corrupt-example` (corrupted `getHealth` example → copied `validate-contracts.cjs` fails on `Health`), `negative-invalid-timestamp` (Feb-30 `expires_at` → copied `validate-contracts.cjs` rejects `BrowserSession`; the `date-time` format is `ajv-formats` full RFC 3339, so timezone-less and calendar-invalid values fail).
- Generated Rust models compile in a temporary crate — `generation` (types-only models + pruned Axum server) and `client-module` (client wire-model module from the same source; generated `REQUIRED_DEPS` selects reqwest 0.13, matching the step-021 pin).
- Orval output typechecks — `generation` (React Query v5 + Zod, strict scratch tsconfig incl. `erasableSyntaxOnly`).
- Operation IDs frozen — `operation-freeze` + `negative-renamed-operation` + committed `scripts/v1-operation-ids.txt`.
- Health server and UI scaffold remain green, future catalog not exposed — full existing gate sweep below plus `contract::catalog_operations_are_not_exposed_by_the_scaffold` alongside the retained `unknown_api_route_is_not_a_success_stub` and `served_spec_is_the_embedded_health_contract`.

Not applicable at this step (no storage until 002): file-backed SQLite/independent pools/controllable clock and revision/history persistence checks; the fixtures already carry actor role, `if_match`, lease and idempotency requirements per operation in `plan/contracts/operations.json`. No new UI controls, so no new UI states/keyboard coverage.

### Test commands and results (Node 24.19.0 via nvm `--node-dir`, Rust 1.98.1, openapi-to-rust 0.17.0)

- `scripts/check-v1-contracts.sh --node-dir ~/.nvm/versions/node/v24.19.0/bin` — exit 0; all named checks green: validate-plan (29 step DAG, 70 operations), validate-examples (108 examples), lint-openapi, lint-asyncapi, contract-smoke (69 schemas, 107 inline examples), operation-freeze (70 IDs), negative-broken-ref, negative-corrupt-example, negative-invalid-timestamp, negative-renamed-operation (each rejected, exit 1), client-module (client.rs + types.rs compile), warm-generation-deps, generation (`plan/check-generation.py` PASS).
- `python3 plan/validate.py`, `node plan/validate-contracts.cjs` — PASS (also run standalone).
- `node --run typecheck` (root, covers the three new `.ts` scripts), `node --run lint:spec`, `shellcheck scripts/*.sh` (0.11.0) — PASS.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` (11 tests incl. the new sweep), `cargo test --package shepherd-server --test api -- contract` (3 tests, exact CI filter) — PASS.
- `npm run generate:api/lint/fmt:check/typecheck/build --prefix ui`, `test:unit:cov` (128/128) — PASS.
- `scripts/hurl-e2e.sh` (2/2 incl. the live boundary negatives), `scripts/smoke.sh` (OK), `scripts/playwright-e2e.sh` (7/7) — PASS.

### Limitations and temporary interfaces

- Cold caches need network once: the generated client's `REQUIRED_DEPS` (reqwest-middleware et al.) are not in a fresh cargo registry, so the gate does `cargo fetch --offline || cargo fetch` before its offline checks; first CI run also pays `cargo install openapi-to-rust` before rust-cache warms.
- Non-GET catalog methods return 405, not 404, on the scaffold: the static-file fallback serves only GET/HEAD. The sweep test pins this truthfully; both statuses prove non-exposure and step 015 replaces the routing anyway.
- `scripts/contract-fixtures.ts server-config` intentionally duplicates the server generator config inside `plan/check-generation.py` (read-only handbook file); a generator bump must update both.
- No temporary code paths were added to application crates; the only Rust change is a test.

Next eligible step: 002.
