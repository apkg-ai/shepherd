# PR review 2026-10-25 — v1-014 backup and restore

Full review of `v1-014-backup-restore` (diff `main...HEAD`, commit e697c2e "Add offline backup and restore maintenance subcommands"): `core/shepherd-core/src/storage/maintenance.rs` (new), `core/shepherd-core/src/storage/mod.rs`, `core/shepherd-server/src/main.rs`, `core/shepherd-core/tests/v1_portability.rs` (new), `core/shepherd-server/tests/boot.rs`, `scripts/{hurl-e2e,playwright-e2e,smoke}.sh`.

Scope: bugs, security, performance. All findings are against code introduced by this PR. Fix commit on the branch: see git log for `[v1 014] Review fixes` on the review-fix commit.

| # | Finding | Where | Severity / class | Disposition |
|---|---|---|---|---|
| 1 | `restore` accepts a manifest with no `shepherd.db` entry: the live DB (and credentials) move to the recovery directory, nothing replaces the DB, and the next `serve` boot provisions a fresh empty database — silent data wipe. `open()` treats a missing DB as a fresh install (`connect.rs` `HeaderProbe::Missing`), and the replay key surviving restore makes that path reachable. | `maintenance.rs` `validate_manifest_names` | High — data-loss bug | Fixed: manifest must contain exactly one `shepherd.db` entry; duplicate entries rejected. Regression tests `restore_rejects_manifest_without_database`, `restore_rejects_duplicate_manifest_entries`. |
| 2 | Verify-then-copy TOCTOU: checksums and the SQLite integrity/schema check run against the source backup, but the staged copy is never re-verified; files changing in the source between verification and copy would land unverified. | `maintenance.rs` `stage_and_swap` | Medium — integrity gap | Fixed: staged files are re-hashed against the manifest before any live file is moved. |
| 3 | The staged DB is written with `fs::copy`, which does not flush; a power loss right after a "successful" restore could leave a truncated `shepherd.db` while the recovery copy holds the intact original. | `maintenance.rs` `stage_and_swap` | Medium — durability | Fixed: staged DB is `sync_all`'d after copy (credentials already fsync via `write_secret_file`). |
| 4 | If recovery-directory creation fails (e.g. same-second restore collides with an existing `recovery-<ts>`), the staging directory — a full copy of the backup including credentials — is left behind in the data dir. | `maintenance.rs` `stage_and_swap` | Low — leak on failure path | Fixed: staging dir removed before propagating the error. |
| 5 | README quickstart still runs the bare binary; the clap rewrite requires a subcommand, so the documented command now exits with a parse error. | `README.md` | Doc bug | Fixed: quickstart now `-- serve`. |

Considered and accepted (no change): `conn.close().await.ok()` ignores close errors on read-only connections; `sync_dir` after the swap is best-effort; `output.exists()`/`create` race resolves to a plain create error; recovery rollback is best-effort by design; `fs::read` of manifest/credentials has no size cap (local offline tool, owner-controlled input); backup copies a symlinked credential source only if one exists inside the 0700 data dir, which the owner already controls. Performance: `VACUUM INTO` for a single-file snapshot and streaming SHA-256 with an 8 KB buffer are appropriate; hashing the staged copy adds one pass over the DB, negligible next to the copy itself.

Verification: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` (22 suites, including 14/14 in `v1_portability` and the boot tests) — all green from `core/` after the fixes.

## 2026-10-25 round 2 — CodeRabbit findings on PR #98

CodeRabbit posted 8 inline comments (6 against the pre-fix code, 2 against the first fix commit). Triage and disposition:

| # | Finding | Already fixed by round 1? | Disposition |
|---|---|---|---|
| 1 | Backup does not sync the output directory or its parent before reporting success (power loss can lose the reported backup) | No | Fixed: `sync_dir(output)` and `sync_dir(parent)` after the manifest write, errors propagated |
| 2 | Manifest must require exactly one `shepherd.db` entry and reject duplicates | Yes (round 1) | Kept |
| 3 | Manifest credential entries must exist exactly when those files exist in the backup dir (silent credential drop/boot otherwise) | No | Fixed: `verify_manifest_shape` before checksums; regression test `restore_rejects_manifest_mismatched_with_backup_contents` |
| 4 | Staged DB not fsynced; `sync_dir` errors discarded | Partly (fsync done in round 1; sync error propagation not) | Fixed: post-swap syncs of the recovery dir and data dir propagate via new `StorageError::PostSwapSync` with a message stating the live files are already replaced |
| 5 | Staging dir leaks when recovery-dir creation fails; recovery name collides within one second | Partly (leak fixed in round 1; name resolution not) | Fixed: recovery name now carries a 4-byte random nonce suffix |
| 6 | Successful-restore test did not prove persisted identity, project survival, or idempotent replay | No | Fixed: `restore_preserves_completed_state_and_credentials` now seeds a plan claim with a retained idempotency key, asserts `ensure_owner` reports `created: false` with the same actor id, the seeded project survives, and the claim retry replays the original response with no duplicate claim or event rows |
| 7 | Round-1 duplicate detection used entry count, missing `[shepherd.db, owner-token, owner-token]` (duplicate with only three entries) | No (introduced in round 1) | Fixed: name-based `HashSet` detection; regression test reshaped to the three-entry case |
| 8 | Rollback after a partially failed swap left already-installed staged files in place (mixed old DB + new credentials) | No | Fixed: installed files are tracked and removed before restoring the recovery copies |

Verification round 2: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` (exit 0; `v1_portability` 15/15) from `core/`.
