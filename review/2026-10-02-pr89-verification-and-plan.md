Verification of PR #89 (branch `v1-007-identity-and-permissions`, head `98677bf`) on 2026-10-02: challenge of the open review threads, a full local run of every quality gate, and an adversarial round-three pass over the diff. Follows the same severity scale as [2026-09-27-full-codebase.md](2026-09-27-full-codebase.md); "new" findings are those not already recorded there or in the step-007 follow-up rounds.

## Gates run locally (all passing)

Commands replicate CI exactly (Node 26 from `.nvmrc`, exact semgrep config and exclusions, `shellcheck scripts/*.sh`).

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` — 416 tests in 14 suites, 0 failed.
- `node --run generate:api` — no diff against the tree; oxlint, oxfmt --check, `tsc -b`, vitest with coverage (128 passed), `vite build` all clean.
- Spectral 0 errors; semgrep with the exact CI rule set and exclusions — 0 findings; shellcheck clean; `cargo audit` clean (1 allowed yanked warning); `npm audit` 0 vulnerabilities.
- Contract tests (5 passed), `scripts/hurl-e2e.sh` (3 files, 17 requests), `scripts/smoke.sh`, `scripts/playwright-e2e.sh` (7 passed) all green; CI is green on all 21 checks at `98677bf`.

## Challenge of the open PR threads

Six threads were unresolved on GitHub when this review started; every one was already fixed in-tree by `b11a404` or `98677bf`, each with its regression test. All six were replied to with the fixing commit and resolved on 2026-10-02 — the PR now has zero unresolved threads and needs no code change for the comments.

- **Authorization fall-through to the cookie** (middleware.rs): fixed — any supplied `Authorization` header is decisive, RFC 7235 case-insensitive scheme, malformed values reject 401 without cookie fallback. Regressions: `bearer_scheme_parses_case_insensitively`, `malformed_authorization_never_falls_back_to_cookie`.
- **CORS preflights bypassed the gate** (lib.rs): fixed — the gate is outermost, cors answers from inside; preflights carry `X-Request-Id`/CSP. Regressions: `allowed_preflight_answers_with_gate_headers`, `preflight_with_untrusted_host_is_rejected_with_gate_headers`.
- **Symlinked staging file in `write_secret_file`** (transaction.rs): fixed — unique unpredictable staging name with `O_CREAT|O_EXCL`, file fsync, rename, parent-dir fsync, cleanup on failure. Regressions: `write_secret_file_replaces_a_symlinked_target_without_following_it`, `write_secret_file_leaves_no_staging_file_behind`.
- **Origin comparison ignored password/query/fragment** (middleware.rs): fixed — `origin_allowed` requires username, password, query and fragment absent; unit cases cover each.
- **Reissue not recoverable across file/DB writes** (identity.rs): fixed for the error-return path — a failed transaction restores the previous token file (`failed_reissue_restores_the_previous_token_file`). The hard-crash window is N-02 below.
- **`list_agents` missing owner check in core** (identity.rs): fixed — `ManageCredentials` enforced in core; regression `list_agents_is_owner_only_in_core`.

## New findings (validated this round)

### N-01 — P3 — `--reissue-owner-token` is silently ignored in diagnostic mode

[`main.rs:75-92`](../core/shepherd-server/src/main.rs#L75): when the replay key is missing on an existing install, the daemon takes the `Bootstrapped::Diagnostic` branch and never consults `config.reissue_owner_token` — the exact restore-from-backup state where an owner would want to rotate a possibly-leaked credential. Nothing is printed; the user believes rotation happened while the old credential stays live. Reissue genuinely cannot run without the key (the codec needs it to seal), so the flag must fail loudly at startup instead of being dropped.

### N-02 — P3 — reissue has a hard-crash window the round-one fix does not cover

[`identity.rs:126-186`](../core/shepherd-core/src/commands/identity.rs#L126): the write order is file-then-commit. A kill or power loss between `write_secret_file` and the commit leaves the file holding the new token while the DB still trusts the old credential; the next boot fails verification. This does not self-heal (unlike `ensure_owner`, whose crash window does). Challenged and accepted as-is: the state is fail-loud, the message names `--reissue-owner-token` as the recovery action, the same flag recovers it, and neither write order (nor a two-file scheme) eliminates the window for a single-user local tool. No code change; record the window in plan/12 so it is a documented decision rather than an unknown.

### N-03 — P3 — Host allowlist is not enforced outside `/api/v1` (`/health`, static UI)

[`middleware.rs:216-229`](../core/shepherd-server/src/middleware.rs#L216): `gate` applies `host_allowed` only on the API branch. `GET /health` answers any `Host`, including in diagnostic mode where the response carries the full reason string with the replay-key file path (main.rs:88-91) — readable by a DNS-rebinding page; static UI assets are likewise served under an attacker-controlled Host. plan/12's Host rule has no API-only carve-out, and the non-API branch still adds `X-Request-Id`/CSP, so the omission looks unintentional. Extend the exact Host check to all paths (Origin stays API-only per its threat model).

### N-04 — P3 — `ensure_owner` bootstraps a second human actor if all human credentials are revoked

[`identity.rs:62-96`](../core/shepherd-core/src/commands/identity.rs#L62): `ensure_owner` probes only for a live credential. A DB holding a human actor whose credentials are all revoked takes the bootstrap branch: a second "owner" actor is created while the first row lives on. Not reachable through the seven routes or the CLI (reissue covers the CLI path via `live_owner_credential_or_actor`); it is a latent hazard for out-of-band DB edits or partial restores. Route the probe through the same `live_owner_credential_or_actor` helper reissue uses, so an existing human actor is reused, never duplicated.

### N-05 — P3 — `Store::command_transaction` is still `pub` (the open half of R-01)

[`transaction.rs:34`](../core/shepherd-core/src/storage/transaction.rs#L34): this PR gated `Store::pool` behind test-support, but the public raw-transaction method still hands any downstream crate a `sqlx::Transaction` over `actors`/`credentials`/`browser_sessions`, bypassing `live_actor`, `require_capability`, `security_audit` and the new identity invariants. It stays `pub` only because the integration tests (`tests/v1_*.rs`) seed actors through it. Finish R-01: give test-support a `seed_actor` (or `raw_seed`) helper the tests use, then make `command_transaction` crate-private with the same `cfg(any(test, feature = "test-support"))` gate as `pool`.

### N-06 — P3 — dead-code warning in plain builds: `workflow::task::complete`

[`task.rs:134`](../core/shepherd-core/src/workflow/task.rs#L134): `pub(crate) async fn complete` is used only by tests today (step 010 will reuse it), so `cargo build -p shepherd-server` warns while `cargo clippy --all-targets` (the CI gate) passes. Pre-existing on origin/main and untouched by this PR, but it keeps plain builds non-warning-clean. Fix with `#[cfg_attr(not(test), expect(dead_code))]`-style gating (must not fire in test builds where it is used) or by moving the function to its step-010 consumer.

## Status of the 2026-09-27 full-codebase review

Fixed by this PR: R-02 (exact Host/Origin allowlists — modulo N-03), R-03 for the one replayable 007 command (`revokeAgent`; the remaining commands are step 015 scope), R-05 half (`bearerFormat: JWT` corrected to opaque tokens), R-08 (plan/12 CSP served on every response). Still open, verified unchanged in the current tree: R-01 remainder (N-05), R-04 (fake RateLimit headers + advertised 429 on /health), R-05 remainder (`servers: https://localhost:7437` vs plain HTTP), R-06 (list_tasks temp-B-tree sort), R-07 (quadratic event scan in epic completion), R-09 (fuzz job is a success-shaped placeholder), R-10 (oversized hand-written modules; `commands/identity.rs` is a new ~1,300-line instance), R-11 (dependency freshness), R-12 (README still claims "step-000 scaffold, health-only daemon"), R-13 (toast dismiss timer restarts on each leave/blur, leaked overwritten timers).

## Remediation plan

Every item below names the exact change, the regression that gates it, the gates to run, and the commit message. Status markers: [open] until landed, then edit this file to [fixed <sha>]. Every code commit follows AGENTS.md: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` (UI items add oxlint/oxfmt/`tsc -b`/vitest/`vite build`; server items add boot/hurl; spec items add spectral/contract), exit codes checked directly, never through a `tail`/`grep` pipe.

Sequencing:

| Track | Scope | Branch | Items |
|---|---|---|---|
| A | Close the open PR#89 follow-ups | `v1-007-identity-and-permissions` | A1-A5 |
| B | Small pre-existing fixes, next PRs against main | one branch each | B1-B5 |
| C | Scheduled with the step that owns the code | owning step | C1-C5 |

### Track A — before PR #89 merges (one gated commit each, in order)

A1 — [fixed b53b88b] **N-01, fail loudly when `--reissue-owner-token` cannot run.** In `core/shepherd-server/src/main.rs`, add a `None if config.reissue_owner_token` arm to the `match provider` that returns an error naming the replay-key path ("cannot reissue the owner token: the replay key {path} is missing; restore it from backup first") instead of silently entering diagnostic mode — the exact restore-from-backup state where the user asked for a rotation and must not believe it happened. Regression in `tests/boot.rs`: existing DB, no `replay-key` file, flag passed → nonzero exit, output names the replay key; the same boot without the flag keeps today's diagnostic behavior. Gates: boot + unit + clippy. Commit: `Fail loudly when reissue cannot run without a replay key`.

A2 — [fixed 9cbed32] **N-03, enforce the Host allowlist on every served path.** In `core/shepherd-server/src/middleware.rs`, hoist the exact Host check out of `api_gate` to the top of `gate`, before the API/non-API branch, so `/health` and static UI assets reject untrusted Hosts; the 400 `malformed_request` response must flow through the shared header insertion so it still carries `X-Request-Id` and CSP. Origin checks stay API-only (their threat model is credential-bearing API traffic). Regressions in `tests/api.rs`: `/health` and `/` with `Host: attacker.example` → 400 with `X-Request-Id` + CSP; `/health` with an allowlisted Host stays 200. Re-run hurl, smoke and playwright to prove no fixture sends an off-allowlist Host. Commit: `Enforce the Host allowlist on every served path`.

A3 — [fixed 1b154f9] **N-04, never create a second owner actor.** In `core/shepherd-core/src/commands/identity.rs`, `ensure_owner` probes via `live_owner_credential_or_actor` instead of `live_owner_credential`: `Some((actor, Some(stored_hash)))` → the existing file-verification path; `Some((_actor, None))` → the `locked_out` problem ("every owner credential is revoked; restart with --reissue-owner-token"); `None` → today's bootstrap branch. Regressions: seed a human actor with only revoked credentials → `ensure_owner` errors and inserts no actor; then `reissue_owner_token` succeeds and the live credential belongs to the same actor id (no duplicate human row, ever). Commit: `Reuse the existing owner actor when every owner credential is revoked`.

A4 — [fixed b2db433] **N-05, finish the R-01 boundary.** Gate `Store::command_transaction` in `core/shepherd-core/src/storage/transaction.rs` with `#[cfg(any(test, feature = "test-support"))]`, mirroring `pool` — the self dev-dependency (`shepherd-core = { path = ".", features = ["test-support"] }`) keeps the integration tests compiling while production builds lose the raw-transaction escape hatch; first verify every in-crate call site (`commands/`, `queries/`, `workflow/` seeders) sits behind `#[cfg(test)]` so nothing production-internal breaks. The production build of `shepherd-server` (which depends on `shepherd-core` without test-support) is the external-crate compile check R-01 asked for: it cannot even name the method. Gates: workspace build + tests + clippy; also `cargo build -p shepherd-server` warning-free. Commit: `Gate the raw transaction API behind test support`.

A5 — [fixed 100105c] **N-02, document the accepted reissue crash window.** No code change. Add one paragraph to `plan/12-security-and-local-identity.md` (local identity section): the reissue write order is file-then-commit; a hard kill between the two leaves the file holding the unusable new token, the next boot fails verification with the message naming `--reissue-owner-token`, and rerunning the flag recovers; the alternative order (commit-then-file) has the symmetric window, so this fail-loud design is deliberate for a single-user local tool. Commit: `Document the owner-token reissue crash window`.

### Track B — small pre-existing fixes, one branch/PR each against main

B1 — [fixed e4d8594] **R-05 remainder, spec transport.** `openapi/shepherd.yaml:18`: `servers` URL `https://localhost:7437` → `http://localhost:7437` (the description already says TLS is post-v1). Regenerate wire types, re-run spectral, contract and hurl gates. Commit: `Advertise the served local HTTP transport in the spec`.

B2 — [fixed 26c93dd] **R-12, README stage.** `README.md` state line still claims the step-000 scaffold, health-only. Update to: core implements steps 001-007 (projects through identity), the served API is the scaffold plus the seven identity operations, current step 007. Keep the core-vs-served-API distinction. Commit: `Update the README implementation stage`.

B3 — [fixed c642a22] **N-06, warning-free plain builds.** `core/shepherd-core/src/workflow/task.rs:134`: add `#[cfg_attr(not(test), expect(dead_code))]` to `complete` (used by tests today, by step 010's execute report later) so `cargo build -p shepherd-server` is silent while `clippy --all-targets` stays green; when step 010 lands, remove the attribute. Gates: plain build of both crates warning-free + full clippy/tests. Commit: `Keep plain builds warning-free for the reserved done transition`.

B4 — [fixed 54183d8] **R-13, toast pause logic.** `ui/src/components/Toast.tsx`: track hover and focus per toast (a boolean pair or counter); `startTimer` clears any existing timer before arming a new one (fixes the overwritten-timer leak where an uncleared timer dismisses a toast that is still paused), and the auto-dismiss timer restarts only when neither hover nor focus holds the toast. Regression: mixed pointer/keyboard test — focus the dismiss button, hover, move the pointer away → the toast stays while focused; blur → dismissed after the full window; repeated leave/blur events arm exactly one live timer per toast. Gates: vitest + oxlint/oxfmt + tsc + build. Commit: `Hold toasts open while hover or focus holds them`.

B5 — [fixed a8bb3c4] **R-04, stop advertising an unenforced quota.** plan/12 says "No broad fake RateLimit headers"; `/health` emits constant `1000/999/60` and the spec advertises a 429 the server never sends. Recommended: remove `health_rate_limit_headers` from `core/shepherd-server/src/middleware.rs` and the `RateLimit-*` header refs plus the `429` response (and the `Retry-After` header schema if nothing else references it) from the `/health` operation in `openapi/shepherd.yaml`; regenerate and re-run spectral/contract/hurl. Alternative (only if the owner wants a real limiter): a fixed-window in-memory counter with a genuinely decreasing `RateLimit-Remaining` and a real 429 + `Retry-After`, with tests — more moving parts for no local-first threat. Commit: `Drop the unenforced health rate-limit advertisement`.

### Track C — scheduled with the step that owns the code

C1 — [fixed ec1c870] **R-06, page-shaped task index.** New migration (never the applied baseline): an index matching the unfiltered page order — `tasks(project_id, archived, created_at, id)` — so `list_tasks` stops using a temp B-tree per page; benchmark the `status IN (...)` work-list shape before and after with realistic distributions and assert `EXPLAIN QUERY PLAN` shows no `USE TEMP B-TREE` in a regression test. Owner: performance acceptance step.

C2 — [fixed c36e092] **R-07, quadratic epic-completion scan.** Replace the `events.iter().any` rescans in `core/shepherd-core/src/workflow/mod.rs` with a set keyed by resource id, and add a 1,000-2,000-deep dependency-chain benchmark before accepting the cascade architecture. Owner: performance acceptance step.

C3 — [fixed b910981] **R-09, real fuzz coverage or an honest job name.** Either wire bounded `cargo-fuzz` targets (cursor codec, token/digest round-trips, AES-GCM codec, graph validation) into the `core-test-fuzz` job with a fixed seed and step cap, or replace the placeholder with deterministic randomized property tests in the existing suites and rename the job so CI status implies nothing it does not do. Owner: test-infrastructure follow-up recorded in the workflow.

C4 — [fixed 9070432] **R-10, module splits.** Split `commands/identity.rs` (~1,300 lines with tests) into `bootstrap.rs` (ensure_owner/reissue), `agents.rs` (create/revoke/list) and `sessions.rs` (login/logout/CSRF), each keeping its test module with its subject; the deferred splits from the step-006 handoff (`commands/hierarchy.rs`, `workflow/eligibility.rs`, `queries/hierarchy.rs`, `commands/dependencies.rs`) land with the steps that next touch them. Public behavior and tests unchanged. Owner: spread across upcoming steps.

C5 — [fixed f4d4289] **R-11, dependency freshness.** One separate tested change: `npm outdated`/`npm update` for root and `ui/`, `cargo update` for `core/`; preserve the deliberate `jsonschema = "0.49"` and `lodash` override pins; full gate run after. Owner: rolling maintenance PR.

### Acceptance

All gates green at every landing commit (the gate set in the header of this plan); each landed item is marked [fixed <sha>] here; PR #89 merges with zero unresolved threads and A1-A5 in its history or a recorded follow-up decision; N-02 is documented in plan/12 so no future review re-raises it.

## Post-implementation notes (2026-10-02)

All three tracks landed on `v1-007-identity-and-permissions` with per-item gated commits and a full-suite verification between tracks (Rust fmt/clippy/workspace tests, UI lint/fmt/tsc/vitest/build, spectral, semgrep, shellcheck, cargo/npm audit, contract, hurl, smoke, playwright). Two C-track scope notes: the deep-chain test is a 100-deep correctness regression, not a timing benchmark (the perf benchmark stays with the performance acceptance step per C1/C2), and the dependency refresh bumped lockfiles within existing ranges — `msw` 2.15→3.0 and `oxfmt` 0.68→0.71 are major version bumps left as deliberate upgrade decisions. The old `identity.rs` file-size finding is resolved by the C4 split (production files now 137-189 lines).
