# 002 — Fresh database and command infrastructure

Status: implemented on branch `002-storage-foundation` (PR pending). Requirements: DATA-01.

## Objective and prerequisites

Deliver fresh database and command infrastructure. Required completed steps: [001](001-contract-baseline.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/storage/{mod,connect,transaction,rows}.rs`
- `core/shepherd-core/src/model/mod.rs`
- `core/shepherd-core/migrations/20260914000001_baseline.sql`
- `core/shepherd-core/src/lib.rs`

Tests: `core/shepherd-core/tests/v1_storage_foundation.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement `storage` and the shared model primitives directly in their final module paths. Apply `contracts/schema.sql` as the only application baseline; add typed IDs, row conversions, injectable clock, SQLite settings, `command_lock` write reservation and transaction helper. Reject the archived MVP path and foreign/nonempty databases without changing them. Implement the idempotency row-codec interface with a test key provider; production encryption arrives in the identity step.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Fresh file opens with application/schema identity; old/nonempty foreign database is rejected without modified bytes. Failure injected after insert rolls back. Two file-backed pools serialize commands; in-memory tests alone are insufficient.

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

Branch: `002-storage-foundation` (off `main` at `b936d11`), five commits `502c7c6..7425466`, SSH-signed. PR/CI pending at handoff time; all step-required commands pass locally (exact results below).

### Files changed

- `core/shepherd-core/Cargo.toml` — retained-stack dependencies re-added: sqlx =0.9.0 (default-features off; `runtime-tokio`, `sqlite-bundled` — deliberately not the `sqlite` umbrella, which would also compile load-extension/unlock-notify/deserialize — and `migrate`; no TLS/any/macros/json), uuid 1.26.1 (`v7`), chrono 0.4.45 (`clock` only), thiserror 2.0.20. Dev-deps: tokio (macros, rt-multi-thread, time), tempfile, sqlx/chrono/uuid mirrors for the integration suite, and a `test-support`-feature self-dependency so integration tests see the gated test helpers. All versions verified as latest stable on crates.io on 2026-09-19.
- `core/shepherd-core/migrations/20260914000001_baseline.sql` (new) — line-for-line copy of `plan/contracts/schema.sql` with only the header comment swapped; `PRAGMA application_id` runs inside the migration transaction so identity appears atomically with the schema; guarded by `baseline_migration_matches_contract_schema`.
- `core/shepherd-core/src/model/mod.rs` (new) — typed UUIDv7 IDs (`ActorId`, `CommandId`; later steps add their own), `Revision` (INITIAL=1), `ActorKind`/`Actor` with explicit SQLite string conversion, `Clock` trait + `SystemClock` (millisecond-truncated so stored RFC 3339 round-trips exactly) + gated `TestClock` (new/set/advance).
- `core/shepherd-core/src/storage/mod.rs` (new) — `Store` facade (pool + `Arc<dyn Clock>` + `Arc<dyn IdempotencyCodec>`), `StorageError` (thiserror; `MvpDatabase`/`ForeignDatabase`/`SchemaMismatch`/`Corrupt`/`Codec`/`Sqlx`/`Migrate`/`Io`), idempotency codec boundary (`IdempotencyAad` = actor/key/request-hash per plan/12, `SealedResponse`, `ReplayKeyProvider`, `IdempotencyCodec`), and `#[cfg(any(test, feature = "test-support"))]`-gated `TestKeyProvider`/`TestCodec` (reversible keyed XOR + tag; wrong AAD or tampering fails like GCM auth).
- `core/shepherd-core/src/storage/connect.rs` (new) — `StoreOptions` (db path, injectable MVP path defaulting to `~/.shepherd/shepherd.db`, clock, codec) and `open()`: MVP-path guard with zero I/O → raw 16-byte header probe (never opens non-SQLite bytes) → read-only SQLite probe of `PRAGMA application_id` (reads through a live WAL; never writes; any probe failure rejects as foreign) → RW pool (max 8 connections, 5 s acquire, WAL, synchronous=FULL, foreign_keys=ON, busy_timeout=5000) → embedded migrator (`Migrator::with_migrations` + `include_str!`; release artifacts need no migrations directory) → `schema_meta`(1, '2.0.0') + application_id verification.
- `core/shepherd-core/src/storage/transaction.rs` (new) — `Store::command_transaction`: BEGIN, immediately `UPDATE command_lock SET value=value WHERE id=1` (write reservation before any reads), closure, single commit; any `Err` rolls back. Closure type is a boxed `TxFuture<'t, T>` rather than `AsyncFnOnce`: spawned callers trip rustc's "implementation of `AsyncFnOnce` is not general enough" higher-ranked inference limit (reproduced; explicit argument annotation does not help).
- `core/shepherd-core/src/storage/rows.rs` (new) — fixed-width RFC 3339 UTC helpers (`format_ts`/`parse_ts`, 24 chars, lexicographic == chronological so the TEXT `expires_at > ?` comparison is correct), `parse_uuid`/`parse_flag`, `SchemaMetaRow`/`read_schema_meta`, actor insert/get (FK plumbing; registration commands are step 007), `IdempotencyRecord` with `put_idempotency`/TTL-aware `get_idempotency`. Runtime `sqlx::query` + hand-typed conversions throughout; UUIDs/timestamps stored as TEXT (STRICT tables), no sqlx uuid/chrono encoders.
- `core/shepherd-core/src/lib.rs` — module declarations; retained `version()`.
- `core/shepherd-core/tests/v1_storage_foundation.rs` (new) — acceptance suite below, public API only, persisted-state assertions through an independent read-only connection.
- `core/Cargo.lock` — resolved additions.

### Acceptance mapping

- Fresh file opens with application/schema identity — `fresh_database_opens_with_v1_identity` (application_id=1397248068, schema_meta (1,'2.0.0'), command_lock seed, baseline row in `_sqlx_migrations`, via independent connection) plus `reopening_v1_database_preserves_data_without_new_migrations`; unit level: `fresh_database_initializes_identity_and_sqlite_settings` (also pins WAL/foreign_keys/synchronous=FULL/busy_timeout=5000 and the exact 21-table set), `empty_file_initializes_fresh_database`.
- Old/nonempty foreign database rejected without modified bytes — `foreign_nonempty_database_is_rejected_without_modified_bytes` and `non_sqlite_file_is_rejected_without_modified_bytes` (full byte compare + directory-listing check that no `-wal`/`-shm`/`-journal` siblings appeared), `mvp_database_path_is_rejected_without_open` (distinct `mvp_database` error, zero I/O, file never created).
- Failure injected after insert rolls back — `failure_injected_after_insert_rolls_back` (prior independently committed command survives; failing command's actor and idempotency rows absent; error propagated).
- Two file-backed pools serialize commands — `two_file_backed_pools_serialize_commands`: two independent `open()`s of the same file (the second also exercises the identity probe against a live WAL database), 24 read-modify-write command transactions with a widened race window; exact final counter 24 and observed closure concurrency never exceeds 1. File-backed by construction; in-memory SQLite could not exercise this.
- Idempotency row-codec interface with test key provider — `idempotency_codec_round_trips_and_rejects_tampered_aad` (seal → persist → reload → open; wrong AAD rejected) and unit `codec_rejects_wrong_aad_and_tampering`.
- Controllable clock for TTL — `idempotency_lookup_honors_ttl_with_test_clock` (visible at 7 days − 1 ms, gone at exactly 7 days).
- Migration is the contract baseline — `baseline_migration_matches_contract_schema` (comment/blank-normalized line equality with `plan/contracts/schema.sql`).

### Test commands and results (Rust 1.98.1, Node 24.19.0 for the repo scripts)

- `python3 plan/validate.py` — PASS (29 step DAG, 70 operations), rerun after this document update.
- From `core/`: `cargo fmt --check` — clean; `cargo test --workspace` — 46 tests green (26 shepherd-core unit incl. doc-visible module tests, 10 `v1_storage_foundation`, 9 shepherd-server api, 1 boot); `cargo clippy --all-targets -- -D warnings` — clean; `cargo test --workspace --doc` — clean. Exit codes checked directly, never piped.
- Coverage (exact CI commands + checker): core-unit lcov via `cargo llvm-cov --workspace --lib --bins --ignore-filename-regex 'shepherd-server/'` and core-integration via `--test '*' --ignore-filename-regex 'shepherd-core/|main\.rs|src/generated/'`; `node scripts/coverage-report.ts --report core` → Unit 98.3% (≥95) ✅, Integration 100.0% (≥70) ✅, union 98.4% (≥92) ✅.

### Limitations and temporary interfaces

- osv-scanner/cargo-audit are not installed on the implementation machine; the Dependency Scan workflow covers the new crates on the PR. sqlx 0.9.0 (2026-05-21) postdates the known sqlx RUSTSEC advisories.
- `StorageError` lives in `storage/mod.rs` until `error.rs` (plan/05 module map) exists at its owning step.
- `Store` has no notifier or production key-provider wiring yet: the broadcast channel arrives with events, AES-256-GCM + `<data-dir>/replay-key` in step 007 replace the `test-support`-gated `TestCodec`/`TestKeyProvider` (compile-time-guaranteed absent from production builds).
- `command_transaction` performs only reservation + closure + single commit; the full plan/05 algorithm (replay lookup, revocation check, recompute cascade) accretes in steps 003–010. Actor row helpers are FK plumbing only.
- `sqlx::migrate!` was deliberately not used (would require the `macros` feature); `Migrator::with_migrations` computes the same checksums, so history stays compatible if the macro is adopted later.

### Review follow-ups (adversarial review on PR #83, commits `99d430f..fbd2c40`)

Two review passes (independent xhigh code review + manual challenge against SQLite's WAL semantics) produced these fixes, all inside the step's owned files:

- **Crash recovery**: a read-only SQLite probe cannot recover a hot WAL, so a crashed v1 database was misclassified as `foreign_database` on restart. The identity check is now raw-header-only (100-byte read, application_id at offset 68) — verified empirically that any read-only SQLite open of a WAL-adjacent file can create `-shm` siblings, so foreign files are rejected from raw bytes with zero SQLite involvement. Fresh initialization checkpoints (`wal_checkpoint(TRUNCATE)`) immediately after the baseline so the header carries the id from first open. Residual accepted window: a crash during the very first initialization before that checkpoint leaves a header without identity and the file is refused as foreign.
- **Reject before write**: `schema_meta` is now inspected over a read-only connection before the RW pool/migrator touch an identified database; a future schema version returns the designed `SchemaMismatch` without modifying the file ("old binaries reject newer schema", plan/13). A failed read-only inspection (recovery needed) falls through to the legitimate RW recovery. Own-database rejections may leave `-shm`/`-wal` siblings from the read-only probe (normal SQLite reader behavior); the data file is proven byte-identical.
- **Idempotency key reuse**: `put_idempotency` deletes a same-key row whose `expires_at` has passed before inserting, so an expired key is reusable instead of permanently poisoning `PRIMARY KEY(actor_id, key)`; a live row still conflicts loudly (replay/conflict semantics are step 010). Write helpers (`insert_actor`, `put_idempotency`) now take `&mut SqliteConnection` per plan/05's "command helpers accept the existing transaction"; reads stay generic over `Executor`.
- **Reservation invariant**: `command_transaction` errors with `Corrupt` if the `command_lock` no-op UPDATE affects ≠ 1 row instead of silently proceeding without write serialization.
- **Test codec robustness**: zero-length nonce BLOBs now fail as `Codec` errors instead of a divide-by-zero panic.
- **Clock/ID precision**: `TestClock::advance` truncates to milliseconds like `new`/`set`; UUIDv7 generation uses a shared `ContextV7` (mutex-wrapped; not `Sync`) so same-millisecond IDs are monotonic.
- **MVP guard**: path comparison additionally canonicalizes both sides when they exist, catching `..` segments and symlinks; the raw-header identity check independently protects the MVP file (it lacks the v1 application_id) even when the path guard misses.
- **Cleanup**: `pool.close()` on `open()` error paths; shared `storage::testing` fixture module (test-support-gated) replaces four duplicated test store builders.
- Rejected review finding: the 5-second `busy_timeout` write-queueing ceiling is the plan/05 specification, with 503 `storage_busy` + client retry as the designed overflow behavior (plan/07).

New named regression tests: `crashed_v1_database_recovers_on_reopen`, `foreign_wal_database_is_rejected_without_modified_bytes`, `foreign_database_with_stray_wal_is_rejected_without_byte_changes`, `newer_schema_meta/`database_is_rejected…`, `unknown_future_migration_is_rejected`, `matching_id_without_schema_meta_is_rejected_without_byte_changes`, `truncated_sqlite_header_is_rejected_without_byte_changes`, `fresh_database_header_carries_identity_before_close`, `expired_idempotency_key_is_reusable_and_live_key_conflicts`, `missing_command_lock_row_fails_loudly`, `mvp_database_indirect_path_is_rejected`, `same_millisecond_ids_are_monotonic`, `test_clock_advance_keeps_millisecond_precision`, plus an empty-nonce codec rejection case.

Post-review results: `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` 62 tests green (39 shepherd-core unit, 13 `v1_storage_foundation`, 10 retained server); coverage Unit 98.1% / Integration 100% / union 98.1% via the CI llvm-cov commands + `scripts/coverage-report.ts`.

### Review follow-ups, round 2 (self-review on PR #83, commit `e37d28b`)

Three actionable findings, one question, all adjudicated against the current tree; the earlier CodeRabbit pass (whole-file header read, probe error classification, pre-write schema identity, nonce division-by-zero, reservation row check) was re-verified as already addressed in `99d430f..e37d28b`, with the 16-byte-read suggestion superseded by the 100-byte raw-header probe.

- **Malformed metadata rejected before any write** [P1, reproduced]: any SELECT/decode error on the read-only schema probe previously fell into the recovery fallthrough, so a matching-ID file with a malformed `schema_meta` reached the migrator (`_sqlx_migrations` created, bytes modified) before rejection. The probe now distinguishes read-only connection failure (still the legitimate crash-recovery path) from query/decode failure (new `SchemaProbe::Malformed` → `ForeignDatabase` before `build_pool`). Named tests: `matching_id_with_malformed_schema_meta_is_rejected_without_byte_changes`, `matching_id_with_corrupt_schema_is_rejected_without_byte_changes` (sqlite_master root-page type flag broken at offset 100, file header intact).
- **Checkpoint result verified** [P1, reproduced as `(busy, log, checkpointed) = (1, 1, 0)`]: `wal_checkpoint(TRUNCATE)` reports reader contention in its result row, not as an error, so the raw `application_id` could stay 0 while `open()` succeeded. `checkpoint_identity` now requires `busy=0` and re-probes the raw header; failure returns the new `StorageError::Checkpoint` (the pool's 5 s `busy_timeout` is honored by the checkpoint, so one attempt already waits out transient readers). Named test: `contended_checkpoint_fails_loudly_and_lands_when_readers_release`.
- **Writable pool and row helpers private to core in production** [P2]: `Store::pool()` and `storage::rows` are `pub(crate)` outside `test`/`test-support`, so downstream crates cannot bypass `command_transaction`'s reservation (the plan requires temporary interfaces to be private to core). Verified with a downstream-crate compile probe (`error[E0603]` module is private, `error[E0624]` method is private).
- **Fresh-path TOCTOU narrowed** (question): a file created between the raw header probe and the write open was silently absorbed by the migrator. `ensure_empty_schema` now asserts an empty `sqlite_master` on the fresh path before the baseline runs; the residual probe→connect window (a DELETE-mode foreign file can still have its journal-mode byte switched by the RW connect before rejection) is accepted, as the pre-checkpoint crash window already is. Named test: `fresh_open_rejects_a_file_that_appeared_mid_open`.
- Retracted finding (TestCodec nonce reuse under `NoContext`): UUID 1.26 fills the v7 payload with randomness when the context supplies zero counter bits, so no change was needed.

Post-round results: `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` 66 tests green (43 shepherd-core unit, 13 `v1_storage_foundation`, 10 retained server incl. boot); coverage Unit 98.2% (≥95) ✅ / Integration 100.0% (≥70) ✅ / union 98.3% (≥92) ✅ via the CI llvm-cov commands + `scripts/coverage-report.ts`; `python3 plan/validate.py` PASS.

### Review follow-ups, round 3 (CodeRabbit on PR #83, commit `d79fa64`)

- **Failed fresh init is retryable** [Minor]: a Checkpoint (or any non-foreign) init failure left a baseline-applied file whose id-less raw header every retry rejected as `foreign_database` — permanently unopenable. `open()` now closes the pool first (SQLite's last-connection checkpoint often lands the header), keeps a landed database, and otherwise removes the fresh db and its `-wal`/`-shm` sidecars so a retry of `open` starts clean; a `ForeignDatabase` from the mid-open guard never deletes the foreign file. Rejected the suggested in-`open` checkpoint retry: the pool's 5 s `busy_timeout` already makes each attempt wait out transient readers. Named tests: `remove_unless_landed_keeps_a_landed_database`, `remove_unless_landed_clears_an_unlanded_init_for_a_clean_retry` (simulates the failed-init bytes, asserts sidecar removal and a clean retry), and `contended_fresh_init_is_cleaned_up_and_retryable` — an end-to-end race through `open()` where a read-only reader polls for the `-wal` sidecar (it must snapshot after the WAL conversion, which it must not block, but before the baseline commit), holds across the checkpoint's busy_timeout, and the test asserts the `Checkpoint` error, the sidecar cleanup, and a successful retry; the race retried up to 5 times for CI jitter (10/10 first-attempt hits locally; each hit costs the 5 s busy wait).

Post-round results: `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` 69 tests green; coverage Unit 98.4% (≥95) ✅ / Integration 100.0% (≥70) ✅ / union 98.4% (≥92) ✅.

Next eligible step: 003.
