# 011 — Versioned documents and plan selection

Status: implemented on branch `v1-011-documents-and-submissions` (PR pending). Requirements: CONTENT-01 REVIEW-02.

## Objective and prerequisites

Deliver versioned documents and plan selection. Required completed steps: [010](010-reports-and-retries.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/model/content.rs`
- `core/shepherd-core/src/commands/content.rs`
- `core/shepherd-core/src/commands/reports.rs`

Tests: `core/shepherd-core/tests/v1_documents_and_submissions.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement project/epic/task document creation, immutable revision append, read/list and task-plan selection. Complete report validation against owned document revision IDs and immutable plan/work submissions. Reject execute success without an output revision and plan success without selected plan. Resolve deferred internal interfaces from report step.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

New document has revision 1; stale append returns 412; accepted plan revision remains readable. Changing selected plan clears approval. Claimed execution cannot select another plan. Project/epic documents are retrievable in later sessions; unsafe links and oversized content fail.

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

Branch: `v1-011-documents-and-submissions`. Commits: `4f620b6` (commands, model, report seam resolution, existing-test migration), `d29b4b9` (acceptance suite), `86cb4f9` (in-module unit suites and document event tests), `4b6b5ac` (handoff), plus the review-fix commit on the same branch.

Files changed: `core/shepherd-core/src/model/content.rs` (new: Document/DocumentRevision types, owner/kind enums, content limits, hand-rolled link scheme-safety shared with reports), `core/shepherd-core/src/commands/content.rs` (new: `create_document`, `create_document_revision`, `select_task_plan`, `get_document`, `get_document_revision`, `list_documents`, `list_document_revisions`, shared `validate_task_owned_revisions`/`require_task_plan_revision`/`task_execute_or_review_work` helpers), `core/shepherd-core/src/commands/reports.rs` (plan success reads `tasks.selected_plan_revision_id`; ownership validation moved to content), `core/shepherd-core/src/model/session.rs` (`ReportInput.selected_plan_revision_id` seam deleted; link safety applied to report links), `core/shepherd-core/src/model/mod.rs` + `commands/mod.rs` (module wiring, `PendingEvent::document`, `document.changed` rank 9), `core/shepherd-core/src/storage/mod.rs` (`testing::seed_task_document` removed), `core/shepherd-core/tests/v1_documents_and_submissions.rs` (new acceptance suite, 19 named tests), `core/shepherd-core/tests/v1_reports_and_retries.rs` + `v1_phase_claims.rs` (migrated from the removed seams to the public commands; `seed_accepted_plan` replaced by the public plan flow).

Test commands and results (run from `core/`): `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` all 19 suites pass (322 lib tests incl. 16 content command + 16 content model + 16 report unit tests; 20 documents-and-submissions, 24 reports, 11 phase-claims integration tests); `cargo test --workspace --doc` pass; coverage via `cargo llvm-cov` + `node scripts/coverage-report.ts --dir coverage --report core`: unit 97.0% (gate ≥ 95), integration 93.1% (gate ≥ 70), union 96.9% (gate ≥ 92); `python3 plan/validate.py` PASS. No reproduced limitation; the served contract in `openapi/shepherd.yaml` is untouched — all document operations are core-only until step 015.

Review-fix cycle (max-effort review, 15 findings, all addressed): successful execute reports now require at least one cited finding/handoff revision (`require_output_revision`, plan/04 session rules); `list_documents` checks route membership (404) before cursors like every sibling list, and `list_document_revisions` checks the document before decoding the cursor; `select_task_plan` authenticates (`live_actor`) before route membership; the selected-plan lookup is one shared helper (`content::selected_plan_revision`) compared as parsed UUIDs; `validate_task_owned_revisions` tolerates duplicate ids; `claim_work_predicate` in commands/mod.rs is the single active-work source with an optional phase filter; new migration `20261012000001_documents_page_indexes.sql` adds `documents_project_page` and `document_revisions_page` (contract schema.sql updated; EXPLAIN-pinned by `document_lists_use_their_page_indexes_without_a_temp_sort`); `serde_uuid_plain` and `stored_owner` replaced by the shared `serde_uuid` helper and `DocumentOwner::from_parts`; the phase-claims pinning test appends a third revision after selection so the pin cannot resolve via `latest_revision_id`; stale comments corrected. Declined as reported: relocating document reads to `queries/content.rs` — the step file boundary and the plan/05 module map own them in `commands/content.rs`; the duplicated cursor decode was fixed by sharing `queries::decode_after` instead.

Owner decisions recorded during planning (2026-10-12): terminal unarchived owners accept only `note`-kind documents (isolated in `DocumentKind::allowed_on_terminal_owner`, one line to widen); re-selecting the already-selected revision is a no-op success after the full guard chain (acceptance retained, no revision bump, no event); selection during another actor's plan claim is allowed for any permitted writer — the active-work guard is phase-based (`task_execute_or_review_work`: execute/review claims and pending submissions block, plan claims never); link scheme-safety (http/https only, no userinfo, no javascript/data/file) applies to both document links and report/session links through one shared validator.

Temporary interfaces (next-step dependencies): `select_task_plan` uses `domain_transaction` + `CommandResult<Task>` following the `update_task` precedent because `Task` is not serde-serializable — wire idempotency for Task-returning operations is resolved uniformly in step 015. `Store::report_failpoint` and `complete_task_for_test` remain test-only drivers; `testing::seed_submission`/`seed_claim` remain until step 012's review commands. Step-012 obligation carried over from the 010 handoff: plan rejection must clear both `selected_plan_revision_id` and `accepted_plan_submission_id` with a regression test.

Next eligible step: [012 — reviews](012-reviews.md), which consumes the pending submissions produced here.
