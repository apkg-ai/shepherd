# 021 — Shared Rust REST client

Status: implemented on branch `v1/021-shared-rest-client` (PR pending). Requirements: API-01.

## Objective and prerequisites

Deliver shared rust rest client. Required completed steps: [015](015-rest-integration.md)

Read [execution rules](README.md) first, then:

- [07-rest-contract.md](../07-rest-contract.md)
- [09-cli.md](../09-cli.md)
- [10-mcp.md](../10-mcp.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/Cargo.toml`
- `core/shepherd-client/Cargo.toml`
- `core/shepherd-client/src/{lib,transport,error,streaming}.rs`
- `core/shepherd-client/src/generated/`
- `core/shepherd-client/tests/contract.rs`

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Add shepherd-client workspace crate with generated wire types, reqwest transport, credential file loading and typed methods for operation catalog. Disable proxies/redirects; enforce loopback URL. Implement error enum with Problem and unknown-outcome transport failures, bounded retry using same key. Do not depend on server/core business modules.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Mock/live tests compare request paths/headers/bodies to contract. Token never appears in Debug/logs. Stale revision is not automatically overwritten. Same-key report retry remains one result. Generated models compile with pinned dependencies.

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

- [x] Referenced requirements and every acceptance sentence implemented.
- [x] Schemas, permissions, transitions and clients remain consistent.
- [x] Success and rejection behavior verified at the public boundary.
- [x] Required commands pass; material environmental limitation recorded accurately.
- [x] Temporary code and next-step dependencies documented.
- [x] Handoff below completed; next eligible step linked.

## Implementation handoff record

Branch `v1/021-shared-rest-client`.

Delivered:

- `core/shepherd-client` workspace crate: types-only `openapi-to-rust` 0.17.0 output (`src/generated/`, 94 schemas from `openapi/shepherd.yaml`, config `core/shepherd-client/openapi-to-rust.toml` mirroring the server's) under hand-written `src/{lib,transport,error,streaming}.rs`. No runtime dependency on shepherd-core/shepherd-server; both appear as dev-dependencies only, for the live contract tests.
- Full client-mode generation was rejected deliberately: the generated `HttpClient` constructs `reqwest::Client::new()` internally (proxies and redirects cannot be disabled, violating plan/12), holds the credential as a plain `String`, and retries with `reqwest-retry` exponential backoff instead of the contract's fixed 250/500/1000 ms. Types-only generation plus a hand-written transport satisfies all three mandates.
- `Client` exposes 68 typed methods, 1:1 with the served operation catalog (67 JSON + `get_events`): per-operation list-query structs with contract enums, `CallOptions { idempotency_key, if_match, lease_token }` for commands (If-Match serialized quoted; the client never generates or pre-validates keys/leases — missing headers surface as the server's 428), credential/session issuance ops take no options and are never retried, `get_principal` serves the MCP startup check.
- Transport: base URL restricted to plain http on exactly `127.0.0.1`/`localhost` (no credentials/path/query), `no_proxy()` + `redirect(Policy::none())`, 30 s per-request timeout (none on SSE), bearer and lease sent as sensitive `HeaderValue`s from a `Secret` wrapper with redacting Debug; `load_token_file` refuses group/world-accessible modes and multi-line files. Retry at 250/500/1000 ms only for transport errors and `storage_busy`, mutations only when caller-keyed, identical headers every attempt; exhaustion yields `Error::Transport` with `unknown_outcome() == true`.
- `error::Error`: `Api(Box<Problem>)` / `Transport` / `Usage` / `Decode` with `code()` (`client_transport`/`client_usage` locally — no invented request ids), `status()`, `request_id()`, `unknown_outcome()`, `problem()` matching the plan/09 exit-code table and plan/10 isError payloads.
- `streaming.rs`: resumable SSE consumer over `Response::chunk()` — typed `Change(Event)` frames, heartbeat comments skipped, `resync_required` surfaced with cursor+reason then closed, `last_event_id()` retained in memory and `resume()` reconnecting with `Last-Event-ID` (beats `after`).
- `scripts/regen-generated.sh` now generates both crates' `src/generated/`; `.gitignore` covers the new path; the CI composite action picks both up unchanged.
- `core/shepherd-server/Cargo.toml` dev-dependency trim (out of list, required): `jsonschema_056` now `default-features = false`. Its default `resolve-http` unified `rustls-no-provider` into the one workspace `reqwest` build, which panics at plain-HTTP client construction; server tests only validate locally. Drops ~490 lockfile lines of TLS tree.
- Tests `core/shepherd-client/tests/contract.rs` (27 named cases): live real server on a loopback socket (`axum::serve` + `shepherd-core` test-support fixtures, hierarchy seeded through the client's own methods) plus a scripted capture server for exact-wire and retry-count assertions. Three `live_catalog_*` sweeps exercise every typed method against the live contract (hierarchy commands incl. block/cancel/waive/archive lifecycles and gated-acceptance 409s; documents→plan-claim→selectTaskPlan→report→submission→review/withdraw/release; agents/revocation and browser-session issuance), pinning lib.rs at 98% line coverage in the integration suite. Acceptance mapping: paths/headers/bodies → `wire_create_task_sends_contract_path_headers_and_body`, `wire_update_task_sends_quoted_if_match_revision`, `wire_report_claim_sends_lease_token_header`, `live_full_claim_flow_round_trips_typed_models`, `live_get_principal_returns_actor_for_startup_check`; token redaction → `debug_output_never_contains_token`; stale revision → `stale_if_match_surfaces_revision_conflict_and_leaves_task_unchanged`; same-key report → `same_key_report_claim_replay_yields_single_session`; pinned-deps compile → workspace build plus `generated_models_deserialize_contract_fixtures`. Negative boundary: loopback/redirect/token-mode rejections, 428 precondition tests with persisted-state checks, transport retry exhaustion (4 attempts counted on the wire), unkeyed-mutation single attempt, `storage_busy` retry with byte-identical key, lease 428/409/200 ladder, SSE live/resume/resync/invalid-cursor, mid-frame UTF-8 splits and the 2 MiB unterminated-frame bound.
- Coverage gate: the unit suite in `.github/workflows/quality-gates.yaml` now ignores `shepherd-client/` alongside `shepherd-server/` (the suite is scoped to shepherd-core; client and server are covered by the integration suite). Replicated CI coverage locally: core-unit 95.9% (gate 95), core-integration 92.6% (gate 70), core total union 95.4% (gate 92).
- Commands (core/ unless noted): `scripts/regen-generated.sh` exit 0; `cargo fmt --check` exit 0; `cargo test --workspace` exit 0 (31 suites, incl. the 27 client contract tests); `cargo clippy --all-targets -- -D warnings` exit 0; repo root `python3 plan/validate.py` PASS; `shellcheck scripts/regen-generated.sh` exit 0; both CI `cargo llvm-cov` invocations replicated locally with thresholds met.

- Flake repair surfaced by CI on this branch: `sse_sends_comment_heartbeats_without_ids` paused the tokio clock and relied on auto-advance, which races past sqlx's real-time internals while the liveness recheck's SQLite IO is in flight (stream closes; reproduced twice on CI under llvm-cov, never locally). The test now waits the real 15s heartbeat bounded by a 20s timeout — slower but deterministic.

Limitations and temporary interfaces:

- `exportProject`/`importProject` remain absent from `openapi/shepherd.yaml` (deferred to #97 with steps 014/015), so the client ships 68 operations and no export/import byte-streaming methods; they land with #97 against the restored contract. No other temporary interfaces.

Next eligible steps: 022 (CLI), 023 (MCP); both consume this crate's typed methods.
