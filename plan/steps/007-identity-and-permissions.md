# 007 — Local owner and agent credentials

Status: implemented on branch `v1-007-identity-and-permissions` (PR pending). Requirements: AUTH-01.

## Objective and prerequisites

Deliver local owner and agent credentials. Required completed steps: [006](006-completion-and-blocking.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/model/identity.rs`
- `core/shepherd-core/src/commands/identity.rs`
- `core/shepherd-core/src/storage/transaction.rs`
- `core/shepherd-server/src/middleware.rs`
- `core/shepherd-server/src/main.rs`

Tests: `core/shepherd-core/tests/v1_identity_and_permissions.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Add token hashing/randomness, owner bootstrap, browser sessions/CSRF, agent issuance/revocation, encrypted idempotent responses and advisory daemon lock. Implement capability matrix in core and transport authentication adapter. Replace test identity bypass with explicit test helper unavailable in production builds. Add secret-redacting wrappers and exact Host/Origin validation.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Agent cannot forge owner role in body, approve human review, create agents or weaken gates. Same key/different actor cannot replay another response. Revoked token fails before replay. Bad Origin/Host/CSRF fails. Owner token/replay key have correct file permissions; no secrets in logs.

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

Branch `v1-007-identity-and-permissions` (PR pending).

### Files changed

- New: `core/shepherd-core/src/model/identity.rs` (SecretString redaction, 32-byte token generation, SHA-256 digests with subtle constant-time compare, canonical request hash, the full 15-row plan/12 capability matrix + `require_capability`, identity payload/grant types), `core/shepherd-core/src/commands/identity.rs` (owner bootstrap/verify/`reissue_owner_token`, `create_agent`, `revoke_agent`, browser login/session/logout, `authenticate_bearer`, keyset-paginated `list_agents`, security_audit writes, `IdentityPaths`), `core/shepherd-core/tests/v1_identity_and_permissions.rs`, `core/shepherd-server/tests/identity_http.rs`, `tests/hurl/03-identity.hurl`.
- Amended: `storage/transaction.rs` (`AesGcmCodec`, `FileReplayKeyProvider` with 0600 enforcement, `DaemonLock` on `std::fs::File::try_lock`, `write_secret_file`/`verify_secret_file_mode`), `storage/mod.rs` (codec trait grew raw-AAD `seal_bytes`/`open_bytes` — session CSRF-at-rest reuses the replay key with the session id as AAD; `CredentialFile`/`DaemonLocked` errors; test-support `testing::seed_actor` as the explicit identity bypass), `commands/mod.rs` (`require_owner` replaced by the matrix chokepoint at every call site; `Replay<T>` + `idempotent_transaction` implementing plan/05 step 2 — revocation check strictly before replay lookup, AES-sealed response stored in the same transaction, `IdempotencyConflict` on hash mismatch), `error.rs` (`Unauthenticated`, `IdempotencyConflict`), `queries/mod.rs` (`split_page` crate visibility), `core/shepherd-server/src/middleware.rs` (rewritten: gate middleware with per-request UUIDv7 id, plan/12 CSP, exact URL-parsed Host/Origin allowlists, bearer-wins auth with no cookie fallback, cookie-mutation Origin+CSRF enforcement, task-local `RequestContext`, Set-Cookie emission; problem shaper reshaping generated-validation rejections into contract Problems and rewriting missing `Idempotency-Key` 422→428; exact-allowlist CORS with dev-only credentialed 5173; RateLimit headers scoped to `/health`; `redact_header` + `REDACTED_HEADERS`), `lib.rs` (`AppState`/`ServiceState`, `IdentityApi` impl beside `SystemApi`, router composition), `main.rs` (`--data-dir`/`SHEPHERD_DATA_DIR` with plan/13 defaults, `--dev`, `--reissue-owner-token`; startup order: data dir 0700 → daemon lock → replay key (missing on an existing install ⇒ diagnostic-only, never regenerated) → store with `AesGcmCodec` → fail-loud owner bootstrap → bind; Host/Origin allowlist uses the actual bound port), `openapi/shepherd.yaml` (+7 identity operations and 9 schemas script-converted from `plan/contracts/openapi.yaml` with a machine-checked equivalence diff; `ownerSession` cookie scheme; bearerAuth described as opaque tokens, JWT format dropped), `openapi-to-rust.toml` (+7 operations), `.spectral.yaml` (`owasp:api4:2023-rate-limit`, `owasp:api2:2023-write-restricted`, `no-x-headers` disabled with plan/12 justifications), `core/shepherd-server/Cargo.toml` (REQUIRED_DEPS additions chrono/serde_urlencoded/url/uuid; dev-dep on shepherd-core test-support), `core/shepherd-core/Cargo.toml` (+`aes-gcm =0.10.3` default-features off — resolved and compiled on pinned Rust 1.98.1; fs2 was replaced by std file locking during review, see follow-ups), `tests/hurl/02-scaffold-boundary.hurl` (unauthenticated API paths now 401 before routing), `scripts/{hurl-e2e,smoke,playwright-e2e}.sh` (isolated mktemp `--data-dir`; hurl reads the owner token and passes origin/owner_token variables), `tests/{api,boot}.rs` (real store fixtures, Host headers, boot lifecycle tests).
- No migration: `credentials`, `browser_sessions`, `idempotency` and `security_audit` ship in the baseline schema.

### What was built

First startup writes `<data-dir>/owner-token` (0600 under 0700) before committing the owner actor + SHA-256 credential in one transaction, so a crash between the two self-heals; later startups verify the file against the stored digest constant-time and abort naming `--reissue-owner-token` on absence or mismatch (owner decision: fail loud, explicit rotation, never silent). Owner-only `createAgent` mints a distinct UUIDv7 agent actor whose plaintext token exists once in the response; `revokeAgent` runs through the replay-aware idempotent transaction and atomically marks the actor revoked, closes its credentials, deletes any browser sessions and audits the reason — claim closure joins in step 009 when claims exist (owner decision: no dead extension points). Browser sessions persist in SQLite with hashed cookie tokens and CSRF encrypted at rest under the replay key (AAD = session id); expiry is fixed 12h with no sliding, and GET /session re-serves the decrypted stored CSRF (the `csrf_ciphertext` column exists precisely for that reading of "refreshes CSRF in memory"). All authorization funnels through `require_capability` in core — the 003–006 owner guards now use the same matrix, so no transport can bypass it. The live HTTP surface grew by exactly the seven auth-owned contract operations, with `getPrincipal` as the end-to-end proof route (owner decision; recorded deviation into the nominally step-015-owned `openapi/shepherd.yaml`, `openapi-to-rust.toml` and `.spectral.yaml`). Transport policy is tower middleware around the generated router: authentication happens before routing (unknown `/api/v1` paths answer 401, not 404), and the authenticated principal crosses into the generated trait impls via a task-local pinned by a concurrency regression test.

### Test commands and results (Rust 1.98.1)

- From `core/`: `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` — 405 green (exit code checked directly, never piped); `cargo test --workspace --doc` clean.
- CI llvm-cov commands + `node scripts/coverage-report.ts --dir core/coverage --report core`: Unit 98.4% (≥95) ✅, Integration 93.1% (≥70) ✅, union 98.2% (≥92) ✅.
- `python3 plan/validate.py` PASS; `node --run lint:spec` 0 errors; `node scripts/check-operation-freeze.ts` PASS (70 frozen); `bash scripts/check-v1-contracts.sh --node-dir …` PASS; `shellcheck scripts/*.sh` clean; `semgrep scan --error --config p/rust --config p/default` over core src + scripts — 0 findings; root `node --run typecheck` clean.
- Live boundary: `scripts/hurl-e2e.sh` 3/3 files (03-identity drives login → CSRF/Origin negatives → issuance → agent restriction → 428 without Idempotency-Key → revoke → replay → dead token → logout against a spawned daemon on a fresh data dir); `scripts/smoke.sh` OK; `scripts/playwright-e2e.sh` 7/7 (shell + WCAG AA under the new CSP).
- Acceptance sentences → named regression tests: forging — `agent_cannot_forge_owner_role_in_body`, `agent_cannot_create_or_revoke_agents`, `agent_cannot_weaken_review_gates_via_registry` (task policy patch + registry, revisions pinned via an independent read-only connection), HTTP `forged_owner_labels_never_grant_rights` (422 on forged body fields, 403 on clean body, principal always credential-derived); review independence — `agent_is_denied_human_review_capability` + unit `capability_matrix_matches_plan12_table` (all 15 rows, both columns); replay — `same_key_different_actor_does_not_replay_foreign_response` (public revoke path and `idempotent_transaction`), `gcm_codec_rejects_foreign_actor_aad` (real AES-GCM), `revoked_actor_fails_before_replay_lookup` (stored row survives untouched), `idempotency_conflict_on_same_key_different_request`; revoked tokens — `revoked_agent_token_is_unauthorized_everywhere` plus the hurl lifecycle; transport — `cookie_mutation_requires_allowlisted_origin`, `csrf_token_required_and_exact`, `host_header_must_match_allowlist`, `origin_prefix_tricks_are_rejected`, `invalid_bearer_never_falls_back_to_cookie`, `bearer_wins_over_cookie`, `dev_origin_only_with_dev_flag`, `missing_idempotency_key_is_precondition_required`, `problems_carry_request_id_and_responses_carry_security_headers`, `request_context_survives_concurrent_requests`; files/secrets — `replay_key_files_have_0600_and_dir_0700`, `world_readable_replay_key_is_rejected`, `owner_token_file_has_restrictive_permissions`, boot `no_secrets_in_server_output` (live login, stdout/stderr scanned for token and cookie values), `secret_string_redacts_debug_and_display`, `credential_headers_are_redacted`; TTL — `browser_sessions_are_fixed_ttl_without_sliding` (TestClock 11h/13h) and the HTTP-boundary variant; daemon — `second_daemon_is_refused_by_advisory_lock` (in-process and spawned), `missing_replay_key_enters_diagnostic_only_mode` (health warn, API 503 integrity_failure, key never regenerated), `missing_owner_token_fails_loud_and_reissue_recovers` (spawned binary). Failure paths assert unchanged rows/audit counts through independent read-only connections.

### Limitations and temporary interfaces

- "Agent cannot approve human review" is enforced at the capability-matrix boundary — the review command itself lands in step 012, which must wire `Capability::ReviewHumanPolicy`/`ReviewAgentPolicy` into its transaction and add the command-level regression.
- Revocation does not close claims or recompute availability (owner decision): step 009 extends `revoke_agent` when claims exist.
- `request_hash` rides as an `idempotent_transaction` parameter rather than in `CommandContext`; step 015 may promote it when all commands become replayable. Only `revokeAgent` is replayable here per the contract.
- The problem shaper buffers and reshapes generated-validation rejection bodies; step 015 inherits the same adapter for the remaining 63 operations.
- `list_agents` lives in `commands/identity.rs` (plan/05's module map has no `queries/identity.rs`).
- Micro-decisions applied (flagged in review): wrong/absent Host → 400 `malformed_request`; supplied unlisted Origin → 403 even with a valid bearer; bearer callers get 404 from GET/DELETE `/session`; re-revoking → 409 `terminal`; agent credential on login → 403; login/logout write no security_audit rows (plan/07 literal); `listAgents` includes revoked agents; diagnostic-mode API responses → 503 `integrity_failure`; unknown `/api/v1` paths → 401 before routing.
- `--db` flag, structured logging, graceful shutdown stay with their owning steps (013/025).
- Local environment: Node 24.19 vs the .nvmrc 26 pin (all Node gates run with the 24.19 toolchain); osv-scanner/cargo-audit are CI-only. `aes-gcm 0.10.3` is the RUSTSEC-2023-0042-fixed release.

### Review follow-ups (xhigh review on PR #89)

Full-PR adversarial review after opening; 12 findings, all resolved or explicitly dismissed, each fix its own gated commit.

- **Owner-token reissue left old browser sessions alive** [security, high]: `reissue_owner_token` rotated the credential but a session minted with the stolen token survived up to 12h — the compromise-recovery path did not recover. Reissue now deletes all human browser sessions in the same transaction. Regression: `reissue_invalidates_existing_owner_browser_sessions`.
- **Secret-leak boot test scanned an empty stream** [test fidelity, high]: `spawn_daemon` consumed and dropped child stdout, so `no_secrets_in_server_output` asserted against `""` (and later daemon prints risked EPIPE). The `Daemon` fixture now keeps the reader; the test drains it after kill and first asserts the output is nonempty (`data dir:` line) so it can never pass vacuously.
- **Problem status mapping gaps** [correctness]: `problem_for` matched a phantom `not_eligible`, sent `dependency_cycle`/`scope_mismatch`/`invalid_state` to 500, and the shared response macro wrapped 412/428 bodies in a 500 wire status. Mapping now covers every `DomainError::code()` per plan/07 (unit-tested exhaustively); the macro fallback re-coerces the body so wire status and body always agree; `revokeAgent` names its contract-declared 428 explicitly.
- **Non-atomic secret writes** [robustness]: `write_secret_file` truncated in place; a crash mid-write could leave a partial replay-key that aborts startup. Now same-directory temp file + fsync + rename, and the wrong-size replay-key error names the recovery action.
- **428 detection was coupled to generated rejection text** [robustness]: the missing `Idempotency-Key` rewrite matched the validator's exact field path and wording. The gate now detects the missing header by route (`POST /api/v1/agents/{id}/revoke`) before the generated layer; step 015 generalizes this from the contract catalog.
- **Cleanups**: cookie Max-Age derives from `SESSION_TTL_SECONDS` instead of a literal; the four hand-rolled identity transactions route through `domain_transaction` (a `codec_arc()` accessor lets closures capture the codec) and `commit_or_rollback` is gone; the dead constant-time re-check after the exact-digest SQL lookup was removed (the indexed equality on SHA-256 digests is the check — documented on `credential_actor_by_hash`); `AesGcmCodec` derives the AES key schedule once at construction; `listAgents` passes the caller's limit through `u32::try_from` instead of clamping.
- **fs2 replaced by std file locking** [dependency, owner-approved]: fs2 0.4.3 is unmaintained (last release January 2018, issues/PRs ignored); `std::fs::File::try_lock` — stable since Rust 1.89, same `flock` semantics, an explicit `TryLockError::WouldBlock` — replaces it with zero dependencies. `plan/dependencies.md` and plan/12 updated with the substitution rationale.
- **Dismissed** [owner decision]: `redact_header`/`REDACTED_HEADERS` stay although no logging pipeline calls them yet — the step spec mandates the redaction wrapper and step 025's tracing subscriber is the consumer.

### Review follow-ups, round two (second review pass on PR #89)

Four findings from the follow-up review, each fix gated with its regression.

- **Authorization fall-through to the cookie** [security, P2]: the scheme parsed case-sensitively (`bearer <agent-token>` + owner cookie answered `200 human`) and malformed values silently tried the cookie. The gate now treats any supplied `Authorization` header as decisive: RFC 7235 case-insensitive scheme, and any malformed/unsupported value rejects 401 without the cookie. Regressions: `bearer_scheme_parses_case_insensitively`, `malformed_authorization_never_falls_back_to_cookie`.
- **CORS preflights bypassed the gate** [security, P2]: the outermost cors layer answered `OPTIONS` before Host/Origin checks, so a preflight with an untrusted Host returned 200 without `X-Request-Id` or CSP. The gate is now outermost (cors answers from inside) and passes credential-less `OPTIONS` through only after its Host/Origin checks; every preflight response carries the gate headers. Regressions: `allowed_preflight_answers_with_gate_headers`, `preflight_with_untrusted_host_is_rejected_with_gate_headers`, `preflight_with_untrusted_origin_is_rejected`.
- **Staging file symlink in `write_secret_file`** [security, P2]: the predictable `.tmp` path opened with create/truncate followed a pre-existing symlink. Staging is now a unique unpredictable name created with `O_CREAT|O_EXCL` (which refuses symlinks), and failed writes clean up after themselves. Regressions: `write_secret_file_replaces_a_symlinked_target_without_following_it`, `write_secret_file_leaves_no_staging_file_behind`.
- **Origin comparison ignored password/query/fragment** [security, P3]: `http://localhost:7437?x=1` passed the allowlist. `origin_allowed` now requires the password, query and fragment to be absent (unit cases in `origin_allowlist_is_parsed_exactly`).

### Review follow-ups, round three (third review pass on PR #89)

Eleven findings against the post-round-two tree; ten fixed, one dismissed.

- **npm-audit gate failed open** [gate integrity, P1]: `|| true` plus suppressed stderr meant an operational `npm audit` failure (registry down, bad `--prefix`, missing jq) produced an empty report, zero advisory ids and exit 0. The gate now requires jq up front, trusts npm's exit code only when the report is parseable JSON without a top-level `error`, and otherwise fails with the raw output. A `via` advisory without a `url` now falls back to the package name — and to a sentinel that cannot be allowlisted — instead of vanishing from the id list.
- **Undecryptable session surfaced as 500** [correctness, P2]: a `browser_sessions` row whose CSRF ciphertext no longer opens under the current replay key (mismatched backup restore) returned `internal_error` 500 on every cookie request, so the UI could never fall back to login. Decrypt failure of a matched row now fails authentication (401); the row ages out at its fixed expiry, keeping GET side-effect-free. Regression: `undecryptable_session_fails_authentication_not_the_server`.
- **Problem-shaper framing on body-collect failure** [correctness, P3]: the error branch returned the original parts (stale `Content-Length`) with an empty body; the header is now dropped in that branch.
- **Host comparison was case-sensitive** [correctness, P3]: RFC 9110 host names compare case-insensitively and the Origin path already lowercases; `host_allowed` now uses `eq_ignore_ascii_case`. Regression: `host_allowlist_is_case_insensitive_and_exact`.
- **Cleanups**: `AuthConfig::new` parses the host/origin allowlists once at startup instead of per request (constructors updated across main/tests); `middleware.rs` (633 lines) split by responsibility into `middleware/{mod,config,context,gate,problem}.rs` — a directory module keeping the plan/05 `middleware` entry's public surface, recorded as a map refinement; the two problem+json response constructors merged into one; the misleading "compare as sets" test comment now states the expected list stays sorted.
- **Spectral exemption narrowed** [gate scope]: `owasp:api2:2023-write-restricted` moved from a global disable to a spectral `overrides` entry scoped to `POST /api/v1/session`, so write operations added in steps 008+ keep the security-scheme lint.
- **Dismissed** [owner decision, second time]: `redact_header`/`REDACTED_HEADERS` without a production caller — spec-mandated wrapper, consumed by step 025's logging; the prior dismissal stands.

Next eligible step: [008](008-proposals-and-policy.md).
