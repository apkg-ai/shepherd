# 006 — Epic completion, block, cancellation and archive

Status: implemented on branch `v1-006-completion-and-blocking` (PR pending). Requirements: FLOW-03 CLAIM-01.

## Objective and prerequisites

Deliver epic completion, block, cancellation and archive. Required completed steps: [005](005-dependencies-and-eligibility.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/workflow/{epic,task}.rs`
- `core/shepherd-core/src/commands/{hierarchy,archive}.rs`

Tests: `core/shepherd-core/tests/v1_completion_and_blocking.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement accept/block/unblock/cancel/waive/explicit-complete/archive commands and synchronous topological completion cascade. Add shared revoke-claims helper; it operates even before claim acquisition feature exists. Preserve done descendants on epic cancellation. Archive requires terminal descendants and never removes edges.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Completing last required task makes epic done and unlocks its successors in one transaction. Proposed/blocked epic does not auto-complete. Cancelled task blocks until waived; waiver does not satisfy task dependency. Empty/all-waived epic requires owner complete. Counts still include archived work.

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

Branch `v1-006-completion-and-blocking` (PR pending).

### Files changed

- New: `core/shepherd-core/src/workflow/{epic,task}.rs` (write transitions + completion predicates), `core/shepherd-core/src/commands/archive.rs` (owner archive for project/goal/epic/task), `core/shepherd-core/tests/v1_completion_and_blocking.rs`.
- Amended: `workflow/mod.rs` (the `recompute_affected` stub became the real `recompute(tx, AffectedScope, events, now)` cascade + `dependents_of`), `commands/hierarchy.rs` (block/unblock/cancel/waive for tasks and epics, `complete_epic`, cascade wired into `accept_epic`/`accept_task`, test-support `complete_task_for_test`), `commands/mod.rs` (`append_events` gained the per-command `reason` column value; `PendingEvent::{claim,submission}` ranks 6/7; shared `revoke_active_claims` + `withdraw_pending_submissions` over `ClaimScope::{Task,Epic}`), `commands/dependencies.rs` (epic-level `delete_dependency` seeds the cascade — owner-approved boundary extension), `model/{mod,project}.rs` (`REASON_MAX_CHARS = 2000` matching the ReasonInput contract).
- No new crate dependencies and no migration: block/waiver/cancellation/archive columns, the claims `'revoked'` status and both event types ship in the baseline schema.

### What was built

Plan/04 lifecycle rows as thin guard-chain commands (membership 404 → capability → revision → archived chain → terminal → state → reason validation) over pure-write transitions in `workflow/{task,epic}.rs`; every write, revocation, withdrawal and event shares one transaction. Block (writer capability per plan/12) sets the reason triple and revokes active claims — pending submissions stay pending; epic block revokes every descendant claim and gives each such task its once-per-command bump + `task.changed`. Unblock (owner) clears the triple and its events carry the cleared block reason (plan/08). Cancel (owner) moves task to cancelled/complete with the cancellation triple, revokes claims and withdraws pending submissions; epic cancel batch-cancels nonterminal descendants with the propagated reason and preserves done ones. Waive (owner; cancelled, unwaived, epic live) records the waiver and bumps the owning epic once (completion denominator changed — same rationale as 005's endpoint bumps). `completeEpic` (owner) covers only the empty/all-waived shapes (`explicitly_completable`), rechecks accepted/unblocked/prereqs-done, and seeds its dependents. The cascade is a wave-based worklist over `eligibility::load_epic_snapshots`: an epic completes iff open/active, unblocked, unarchived, every epic prerequisite done and `auto_completable` (>=1 non-waived task, all done); completions re-use a revision already pending from the same command (waive/unblock/accept/dependency bump) so each resource gets exactly one `.changed` with its final revision, then dependents fan out through `epic_deps_reverse` (DAG + monotone ⇒ termination, batched queries per wave). Cascade triggers wired now: accept epic/task, unblock epic/task, waive, explicit complete, epic-level dependency deletion, and the test-support task completion. Archive (owner, `commands/archive.rs`): project/goal require only-terminal contained epics *and* tasks (empty allowed), epic/task must be terminal with no active work (`ActiveWork` otherwise); archive writes the flag + reason triple, bumps once, never touches dependency rows, and counts keep including archived work.

### Test commands and results (Rust 1.98.1)

- `cargo fmt --check` — clean; `cargo clippy --all-targets -- -D warnings` — clean; `cargo test --workspace` — 338 green (exit code checked directly, never piped); `cargo test --workspace --doc` — clean. All from `core/`.
- CI llvm-cov commands + `node scripts/coverage-report.ts --dir core/coverage --report core`: Unit 98.4% (≥95) ✅, Integration 100.0% (≥70) ✅, union 98.4% (≥92) ✅.
- `python3 plan/validate.py` — PASS (29 step DAG; 70 operations).
- `semgrep scan --error --config p/rust --config p/default core/shepherd-core/src` — 0 findings.
- Every acceptance sentence has a named regression test in `tests/v1_completion_and_blocking.rs`: `completing_last_required_task_completes_epic_and_unlocks_successors_in_one_transaction` (multi-hop cascade, single command_id, one coalesced `epic.changed` per epic, successor eligibility flip), `proposed_epic_does_not_auto_complete` (accept then cascades), `blocked_epic_does_not_auto_complete_until_unblocked` (children finished before block; cleared reason preserved), `cancelled_task_blocks_completion_until_waived`, `waiver_does_not_satisfy_task_dependency`, `empty_epic_requires_owner_complete`, `all_waived_epic_requires_owner_complete` (auto never fires on the last waive), `counts_still_include_archived_work` (task and epic archive; edges retained; goal counts) — plus `failed_lifecycle_mutations_leave_revision_and_history_unchanged` (independent read-only connection), `block_and_cancel_revoke_seeded_active_claims` (direct-SQL claims/submissions; submission survives block, withdrawn on cancel), `epic_cancellation_preserves_done_descendants_and_propagates_reason` (downstream still gated), `archive_guards_and_frozen_scope`, `cancel_vs_complete_race_yields_single_winner` (two independent pools on one file-backed database), `creating_task_under_completed_epic_is_terminal` (FLOW-03 at the public boundary), `deleting_last_unmet_epic_dependency_completes_finished_epic`. In-module suites cover every guard branch per command (13 hierarchy, 4 archive, 1 dependencies, 3 plumbing, 2 predicate tests) — the core-unit coverage gate counts only `--lib` tests.

### Limitations and temporary interfaces

- `Store::complete_task_for_test` (`#[cfg(any(test, feature = "test-support"))]`) drives the real `workflow::task::complete` + cascade in one transaction until step 010's `reportClaim` becomes the production driver of the same transition; its event action `system.test.completeTask` sits outside the operationId catalog and never compiles into production builds.
- `revoke_active_claims` revokes every `status='active'` row regardless of expiry; expired-claim reconciliation semantics belong to step 009.
- `claim.changed` events carry the claim row's pinned `task_revision` (claims have no revision column) — revisit when the claim model lands in 009.
- No TTL-dependent behavior exists in these commands, so the controllable clock is exercised only through claim-expiry fixtures.
- Decisions confirmed with the owner during planning: the test-support completion driver (proves the one-transaction acceptance sentence); archive commands for all four resources land here; the cascade is a scoped worklist per the plan/05 `recompute` signature; epic-level `delete_dependency` seeds the cascade (one-line extension outside the declared file list, regression-tested).

Next eligible step: [007](007-identity-and-permissions.md).
