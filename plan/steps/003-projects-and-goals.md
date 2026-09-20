# 003 — Project and goal ownership

Status: implemented on branch `v1-003-projects-and-goals` (PR pending). Requirements: HIER-01 HIER-02.

## Objective and prerequisites

Deliver project and goal ownership. Required completed steps: [002](002-storage-foundation.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/model/{project,goal}.rs`
- `core/shepherd-core/src/commands/hierarchy.rs`
- `core/shepherd-core/src/queries/hierarchy.rs`

Tests: `core/shepherd-core/tests/v1_projects_and_goals.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement project/goal create/list/get/update with revision checks, settings snapshots and explicit ownership. Seed built-in task types per project. Add cursor codec and query filters. Return derived zero counts for empty resources. Use authenticated test actor context; no preview HTTP exposure yet.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Owner can create project and two goals; agent cannot. Empty goal completed=false. Duplicate type key fails; cursor reused with another filter fails; cross-project get returns 404.

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

Branch `v1-003-projects-and-goals` (PR pending).

### Files changed

- New: `core/shepherd-core/src/{error.rs, model/project.rs, model/goal.rs, commands/mod.rs, commands/hierarchy.rs, queries/mod.rs, queries/hierarchy.rs}`, `core/shepherd-core/tests/v1_projects_and_goals.rs`.
- Amended: `lib.rs`/`model/mod.rs` (module declarations, shared `Counts`/`LifecycleRecord`/`EventId`/text validators, `typed_uuid!` re-export), `storage/transaction.rs` (extracted `begin_command` so `domain_transaction` shares the write reservation; `command_transaction` behavior unchanged), `core/shepherd-core/Cargo.toml` + `core/Cargo.lock`, `plan/dependencies.md` (serde =1.0.229 derive, serde_json =1.0.151, base64 =0.23.1 — all versions already in the lockfile).

### What was built

Project/goal create/get/update and full task-type registry parity (seed 6 builtins atomically in createProject; createTaskType/listTaskTypes/updateTaskType, label rename only) as `impl Store` domain commands/queries. `CommandContext`/`CommandResult` per plan/05; inside each command transaction: actor revocation recheck → route membership → owner capability → `expected_revision` (None → `precondition_required`, mismatch → `revision_conflict`) → archived-scope guard → content validity → writes → events (one command_id, ordered resource-kind-then-UUID, same tx). `DomainError` in `error.rs` with stable Problem code strings. Cursor codec: base64url(no-pad) JSON `{v,e,f,c,i}` binding endpoint + normalized filter fingerprint; mismatch/tamper → `invalid_cursor`. Lists: created_at ASC, id ASC keyset paging (limit default 50/max 200, fetch limit+1, next_cursor omitted at end), archived excluded unless `include_archived`, counts via one batched GROUP BY over `epics` (returns zeros now); `Goal.completed` derived (`total > 0 && done == total`).

### Test commands and results (Rust 1.98.1)

- From `core/`: `cargo fmt --check` — clean; `cargo test --workspace` — 115 green (80 shepherd-core unit incl. in-module command/query suites, 12 `v1_projects_and_goals`, 13 `v1_storage_foundation`, 10 retained server); `cargo clippy --all-targets -- -D warnings` — clean; `cargo test --workspace --doc` — clean. Exit codes checked directly, never piped.
- Coverage via the CI llvm-cov commands + `node scripts/coverage-report.ts --report core --dir core/coverage`: Unit 98.7% (≥95) ✅, Integration 100.0% (≥70) ✅, union 98.7% (≥92) ✅.
- `python3 plan/validate.py` — PASS (29 step DAG, 70 operations), rerun after this document update.
- Every acceptance sentence has a named regression test: `owner_creates_project_and_two_goals`, `agent_cannot_create_project_or_goal`, `empty_goal_reports_zero_counts_and_not_completed`, `duplicate_task_type_key_fails`, `cursor_reused_with_different_filter_fails`, `cross_project_goal_get_returns_not_found`; plus `stale_revision_leaves_resource_unchanged`, `update_bumps_revision_exactly_once`, `goal_pages_round_trip_with_cursor`, `invalid_input_is_rejected_without_writes`, `two_pools_serialize_goal_creation` (two independent pools on one file), `include_archived_filter_controls_visibility`. Persisted-state checks use an independent read-only connection; mutation failures proven to leave rows/revisions/events untouched.

### Limitations and temporary interfaces

- Idempotency replay and post-commit notify are not wired: `CommandContext.idempotency_key` is carried but unused until the real codec (007) and events/notifier steps; `domain_transaction` does reservation → checks → writes → events → single commit only.
- `TaskTypePatch` has no `archived` field: archive/unarchive of registry entries is step 008 ("archived type cannot be assigned" semantics land there). Archived entries reject label renames (`ArchivedScope`).
- The archived-scope guard on project/goal/task-type mutations is dormant — no public command can archive until step 006 — and is exercised via direct-SQL fixtures.
- `DuplicateTaskTypeKey` has no contract wire code (plan/07's 409 list lacks a duplicate-key entry); `DomainError::code()` maps it to `validation_error` provisionally, to be settled when step 015 wires HTTP.
- Three `AssertSqlSafe` sites interpolate compile-time column-list consts only (sqlx 0.9 `SqlSafeStr`).

### Review follow-ups (local self-review, pre-PR)

A high-effort local review of `main...HEAD` produced three findings, all fixed on the branch:

- **Archived-parent guard on updates** [medium ×2]: `update_goal`/`update_task_type` only checked the entity's own `archived` flag, so a live child under an archived project could still be renamed (revision bump + event inside archived scope, violating plan/03). Added `queries::hierarchy::project_archived` (one scalar SELECT; `NotFound` when missing) and use it in all four child-scope commands — updates gained the parent check; creates switched to it from `find_project`, dropping a needless counts GROUP BY while keeping membership→capability→archived precedence. `mutations_beneath_an_archived_project_are_rejected` now also asserts live-goal and live-task-type updates under an archived project return `ArchivedScope` with revisions/events untouched.
- **Snapshot reads** [low]: pool-backed `get_*`/`list_*` ran the entity SELECT and the epic-counts GROUP BY as two auto-commit statements, so a concurrent commit could pair revision N with counts from N+1 state. The five Store read wrappers now run in a deferred read transaction (plan/05: query helpers take an Executor or read transaction for one snapshot).

Post-fix results: `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` 115 green; coverage Unit 98.7% / Integration 100% / union 98.7% via the CI llvm-cov commands + `scripts/coverage-report.ts`; `python3 plan/validate.py` PASS.

Next eligible step: [004](004-epics-and-tasks.md).
