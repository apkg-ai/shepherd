# 001 — Contract baseline and generation scaffolding

Status: not started. Requirements: API-01.

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

- [ ] Referenced requirements and every acceptance sentence implemented.
- [ ] Schemas, permissions, transitions and clients remain consistent.
- [ ] Success and rejection behavior verified at the public boundary.
- [ ] Required commands pass; material environmental limitation recorded accurately.
- [ ] Temporary code and next-step dependencies documented.
- [ ] Handoff below completed; next eligible step linked.

## Implementation handoff record

Fill when implementing, not during planning: commit/branch; files changed; test commands and results; any reproduced limitation; temporary interfaces; next eligible step IDs. If an acceptance criterion cannot pass, leave this step incomplete and explain the concrete blocker. Do not mark complete for code that merely compiles.
