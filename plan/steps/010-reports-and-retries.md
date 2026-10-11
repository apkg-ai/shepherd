# 010 — Atomic reports and failure recovery

Status: implemented on branch `v1-010-reports-and-retries` (PR #92 pending). Requirements: DATA-01 CLAIM-01.

## Objective and prerequisites

Deliver atomic reports and failure recovery. Required completed steps: [009](009-phase-claims.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/commands/reports.rs`
- `core/shepherd-core/src/model/review.rs`
- `core/shepherd-core/src/storage/transaction.rs`

Tests: `core/shepherd-core/tests/v1_reports_and_retries.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement failed/successful Session reporting as one command: validate lease, persist session, prepare submission through internal helper, release claim, update attempts/phase, cascade and persist result/events. For this step tests may create document/revision rows through a narrowly scoped test fixture builder until public document commands land in 011; report validation must still use real persisted revisions. Internal submission storage may land here; review decision API lands later. Add deliberate transaction failpoints available only in tests.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Inject failure after lease closure/session insert/task update/event insert: all state rolls back. Same-key successful retry returns original Session; different payload conflicts. Expired/revoked/other-actor report fails. Execution attempts increment once; planning attempts do not increment execution attempt_count.

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

Branch: `v1-010-reports-and-retries` (PR #92). Commits: `cdf196f` (initial implementation), `ea9a59c` (first review pass: dedupe live_actor, batch revision query, did_complete flag), plus the PR-review fix commit on the same branch.

Files changed: `core/shepherd-core/src/commands/reports.rs` (new: `report_claim`, document-revision ownership and plan-kind validation, submission helper), `core/shepherd-core/src/model/session.rs` (new: Session, ReportInput, `validate_report_input` with wire limits), `core/shepherd-core/src/commands/mod.rs` (`session.recorded` event), `core/shepherd-core/src/commands/claims.rs` (claim-row helpers made `pub(super)`), `core/shepherd-core/src/storage/mod.rs` (test failpoints, `seed_task_document` fixture), `core/shepherd-core/src/workflow/task.rs` (`complete` promoted to the production report path), `core/shepherd-core/tests/v1_reports_and_retries.rs` (acceptance suite), `core/shepherd-core/src/commands/reports.rs` in-module unit suite (coverage gate).

Test commands and results (run from `core/`): `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` all suites pass (18 report unit tests, 19 report integration tests); `cargo llvm-cov --workspace --lib --bins` unit line coverage 96.5% (gate ≥ 95%); integration coverage artifact above its ≥ 70% gate; `python3 plan/validate.py` PASS.

PR-review fixes included in the handoff: duplicate `document_revision_ids` are deduplicated (previously a panic), revision ownership is validated for failed reports too, the selected plan revision must belong to a `plan`-kind task document, `summary`/`links`/array-size wire limits from the ReportInput contract are enforced in the core, stray `selected_plan_revision_id` outside a successful plan report is rejected, the dead reconciled-claims event loop was removed (`one_active_claim_per_task` makes it unreachable on the success path), and an independent-pool concurrency regression test was added.

Reproduced limitation: none. The served contract in `openapi/shepherd.yaml` is untouched; reportClaim is core-only until step 015, per the scaffold rule.

Temporary interfaces (next-step dependencies): `ReportInput.selected_plan_revision_id` is a step-010 seam — step 011's `selectTaskPlan` persists the selection on the task and the report then reads it from there (wire `ReportInput` deliberately has no such field, `additionalProperties: false`); `testing::seed_task_document` seeds documents until public document commands land in step 011; `Store::report_failpoint` and `complete_task_for_test` are test-only drivers.

Next eligible step: [011 — versioned documents and plan selection](011-documents-and-submissions.md), which resolves the report step's deferred internal interfaces.
