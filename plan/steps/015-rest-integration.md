# 015 — REST integration and resumable SSE

Status: in progress — REST and SSE delivered; portability wiring deferred to #97. Requirements: API-01 DATA-01.

## Objective and prerequisites

Deliver REST integration and resumable SSE. Required completed steps: [013](013-context-and-history.md), [014](014-portability.md)

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

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Copy `plan/contracts/openapi.yaml` to `openapi/shepherd.yaml` and `plan/contracts/asyncapi.yaml` to `openapi/shepherd-events.asyncapi.yaml`. Replace the health-only scaffold allowlist, generate full server traits and implement every operation handler using the completed core commands. Replace scaffold wire/contract fixtures with stable-v1 fixtures. Add SSE authentication, persisted replay, explicit resynchronization and truthful errors. For staged import, enforce the specified 30-second idle-progress timeout across body receipt and record validation; reset it on each received chunk or validated record, delete staging state on timeout/disconnect, and do not impose a total-duration cap.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Every catalog operation is implemented with no success stubs. Real HTTP tests validate responses and errors against the contract. A stalled import returns 408 `import_timeout` after 30 seconds without progress and commits no project; an import that makes progress more often than every 30 seconds may run beyond 30 seconds and complete. Disconnect also removes staging state. A non-v1 database refuses startup. SSE lost-connection, restart and history-gap scenarios resynchronize correctly. `cargo test --workspace` passes with the stable-v1 suites and no MVP behavior restored.

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

Branch `v1/015-rest-sse` (owner-approved scope split: REST + SSE in this step's PR; `exportProject`/`importProject` deferred to #97 because the owning core surface — step 014's `core/shepherd-core/src/export.rs` — was never implemented).

Delivered:

- `openapi/shepherd.yaml` promoted from `plan/contracts/openapi.yaml` minus the two deferred portability paths (68 operations served); `openapi/shepherd-events.asyncapi.yaml` copied verbatim; root `.spectral.yaml` replaced with the stable profile and `.spectral-asyncapi.yaml` added (`lint:spec` covers both). The trim is pinned by `served_spec_is_the_v1_contract_minus_deferred_portability`; the #97 PR restores verbatim equality.
- Generator allowlist at 67 operations; all generated handlers implemented in `core/shepherd-server/src/handlers/{identity,hierarchy/*,claims/*,content/*,reviews,events}.rs` over `convert/{hierarchy,work,content,errors}.rs` (directory modules keep files small; owner-approved reading of the braces list). Gate derives Idempotency-Key/If-Match/X-Lease-Token requirements from the embedded served spec (missing → 428, malformed → 400); ETag rides the request context; unrouted `/api` paths answer problem 404.
- Core additions owned by this step: `Store` post-commit broadcast notifier (`subscribe_events`, capacity 256), `queries/sessions.rs` (`list_task_sessions`), `queries/views.rs` (`TaskView`/`EpicView` for the wire eligibility fields), `Store::get_actor`; `session.recorded`/`review.recorded` events now carry `resource_revision: 1` (contract minimum; previously 0).
- Handwritten SSE route with Last-Event-ID/after cursors, subscribe-before-replay, 15 s comment heartbeat doubling as revocation/session recheck, `resync_required` (cursor_ahead/cursor_expired/backlog_limit) then close, lag disconnect at the 256-notification bound.
- Tests: `v1_rest_support` (core), `rest_hierarchy`, `rest_workflow`, `rest_claims`, `rest_contract` (`every_implemented_operation_conforms` validates all 67 operations' live responses against the served spec), `sse` (10 scenarios incl. AsyncAPI payload validation), `boot::non_v1_database_refuses_startup`; hurl `02` rewritten as the catalog boundary plus new `04-rest-crud.hurl`. Commands: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `python3 plan/validate.py`, `node --run lint:spec`, UI `generate:api`/typecheck/lint/test, `scripts/hurl-e2e.sh` — all passing at each commit.

Limitations and temporary interfaces:

- Acceptance sentences about staged import (408 `import_timeout`, staging cleanup) are not dischargeable in this PR: no core export/import exists (see #97). The step stays incomplete until that PR lands and this record is finalized.
- Hierarchy-family mutations accept the required Idempotency-Key but do not replay (prior-step deferral, tracked in #99); the replayable family (claims/report/reviews/documents/revokeAgent) has full replay semantics covered by `rest_claims`.
- `tokio-stream` 0.1 added to shepherd-server (already in Cargo.lock transitively) for the SSE channel adapter; owner-approved addition in the fs2→`std::fs::File::try_lock` precedent's spirit, to be reflected in plan/dependencies.md if kept.
- Replayed `claimTask` returns the original lease with a freshly assembled context bundle (the contract only guarantees the lease).
- The non-v1 refusal applies once the install is provisioned: with the replay key missing the daemon enters the plan/12 diagnostic-only mode before ever opening the database, so a foreign file plus missing key serves 503s instead of exiting (`boot.rs` covers the provisioned path).

Next eligible steps: #97 (portability completion, finalizes 014/015), #99 (hierarchy idempotent replay), then 016 (UI shell and hierarchy).
