# 015 — REST cutover and resumable SSE

Status: not started. Requirements: API-01 DATA-01.

## Objective and prerequisites

Deliver rest cutover and resumable sse. Required completed steps: [013](013-context-and-history.md), [014](014-portability.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `openapi/shepherd.yaml`
- `openapi/shepherd-events.asyncapi.yaml`
- `.spectral.yaml`
- `.spectral-asyncapi.yaml`
- `core/shepherd-server/openapi-to-rust.toml`
- `core/shepherd-server/src/handlers/{mod,hierarchy,claims,content,reviews,identity,events,portability}.rs`
- `core/shepherd-server/src/convert/`
- `core/shepherd-core/src/ (v1 namespace promotion)`

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the existing related implementation and targeted tests. Record which functions/queries currently enforce the invariant and which need replacing.
2. Promote contracts/openapi.yaml and asyncapi.yaml into openapi/. Replace generator operation allowlist and generate full server traits. Implement all operation handlers using v1 core commands. Move v1 modules into final backend tree, remove mixed MVP lifecycle/store scaffolding and preview mechanism. Replace old wire/contract fixtures with stable v1 fixtures in the same step. Add SSE authentication/replay/resync and truthful errors.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Every catalog operation is implemented with no success stubs. Real HTTP tests validate responses and errors against contract. Non-v1 DB refuses startup. SSE lost connection/restart/gap scenarios resync correctly. cargo test --workspace passes after old semantic tests are replaced with mapped v1 tests.

For each sentence above create a named regression test with setup → action → expected status/error → persisted state checks. Mutation failures must leave resource revision/history unchanged except an independently committed prior command. Use file-backed SQLite and independent pools for concurrency, controllable clock for TTL; do not test only a mocked helper that mirrors implementation. For UI, cover loading/empty/error plus keyboard interaction for new controls, using MSW for component tests and real server for critical E2E.

## Excluded work

Do not implement subsequent steps or change confirmed product decisions. No external-agent spawning, recursive hierarchy, cross-goal dependency, server-wide auth bypass or MVP data converter. Only add dependencies pinned in [dependency choices](../dependencies.md). Never reset or delete the user's existing database to make tests pass.

## Verification

```sh
# Repository root; activate Node 26 from .nvmrc before npm commands.
python3 plan/validate.py
# core/ (for new Rust behavior)
cargo fmt --check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Run Rust commands from core/, not the repository root. Before step 015, generated MVP sources remain needed for full workspace checks; run scripts/regen-generated.sh if missing, knowing it formats Rust. At step 015 and later generate from promoted v1 contract. For UI generation run npm run generate:api --prefix ui before typecheck when contract changed. Hurl/Playwright need a fresh isolated database and their installed tools; use existing scripts rather than a personal running daemon.

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
