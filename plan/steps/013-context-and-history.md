# 013 — Context assembly and durable history

Status: implemented on branch `v1-013-context-and-history` (PR #96). Requirements: CONTENT-01 DATA-01.

## Objective and prerequisites

Deliver context assembly and durable history. Required completed steps: [012](012-reviews.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/queries/{context,history}.rs`
- `core/shepherd-core/src/model/event.rs`
- `core/shepherd-core/src/storage/transaction.rs`

Tests: `core/shepherd-core/tests/v1_context_and_history.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement snapshot context query with selected plan, relevant documents, recent sessions/reviews and sibling awareness. Add ordered event storage, history pagination and complete event emission coverage. Implement bounded replay query independent from HTTP streaming.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

A different agent receives exact accepted plan and prior feedback, not just latest mutable note. Context truncation is explicit and has continuation links. Every successful command has coherent event group; failed transaction has none. Event IDs survive restart; duplicate delivery has stable IDs.

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

Branch: `v1-013-context-and-history`. Commits: `69c909f` (event read model, rowid cursors, history/replay queries), `71cce9e` (snapshot context query with budgeted truncation + 29-test acceptance suite), `484e621` (in-module context/history unit suites for the coverage gate).

Files changed: `core/shepherd-core/src/model/event.rs` (new: `EventId` moved from model/mod.rs, wire-shaped `Event` with decimal-string id serde rejecting `^[1-9][0-9]*$` violations, 11-variant `EventType`), `src/model/mod.rs` (event module wiring; `Dependency`/`DependencyLevel` gain serde derives — the only model derive additions; `typed_id_serde!(serde_dependency_id)`), `src/model/content.rs` (`serde_project_id` widened to `pub(super)`), `src/queries/mod.rs` (MAC/version/endpoint/filter verification extracted into `verify_cursor`; new `encode_rowid_cursor`/`decode_rowid_cursor` for integer event ids — empty `c`, decimal `i` ≥ 1; UUID and rowid cursors cross-reject), `src/queries/history.rs` (new: `event_from_row`, `list_history` newest-first `id DESC` keyset with optional `resource_id` filter bound into the cursor fingerprint, `Store::event_high_water`, `Store::replay_events` returning `ReplayOutcome::{Events, ResyncRequired{current_cursor, reason}}` with `ResyncReason::{CursorExpired, CursorAhead, BacklogLimit}`, `REPLAY_MAX_EVENTS = 1000`, `REPLAY_WINDOW_DAYS = 7`), `src/queries/context.rs` (new: `ContextBundle`, `Store::get_task_context`, `CONTEXT_BUDGET_BYTES = 262144`, single-pass `BudgetFill`, wire-shaped mandatory measurement, private `session_from_row`), `src/error.rs` (`ContextTooLarge { continuation }` → `context_too_large`), `src/storage/transaction.rs` (`Store::read_transaction` snapshot helper), `src/commands/mod.rs` + `commands/content/mod.rs` + `queries/claims.rs` + `queries/submissions.rs` (`pub(crate)` promotions/re-export of `revision_from_row`, `revision_row`, `claim_from_row`, `review_from_row`), `src/storage/mod.rs` (`testing::seed_submission` fixture correction: `sessions.claim_id` is now a UUID — the literal `'claim'` fails the strict session reader), `tests/v1_context_and_history.rs` (new, 29 named tests).

Test commands and results (run from `core/`): `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` all suites pass (357 lib tests incl. 7 new context/history unit tests and 7 event/cursor model tests; 29 v1_context_and_history integration tests); `cargo test --workspace --doc` pass; coverage via `cargo llvm-cov` + `node scripts/coverage-report.ts --dir core/coverage --report core`: unit 97.1% (gate ≥ 95), integration 93.1% (gate ≥ 70), union 97.0% (gate ≥ 92); `python3 plan/validate.py` PASS. No migration added — `events_project`/`events_resource`/`sessions_task`/`submissions_task_page`/`one_review_per_submission`/`documents_owner` cover every new query; the `migrations == 5` assertion in `v1_storage_foundation.rs` is untouched.

Acceptance sentence → regression tests. Exact plan and feedback: `context_returns_pinned_accepted_plan_not_latest_revision` (pinned `selected_plan_revision_id`, never `latest_revision_id`), `context_includes_prior_reviews_and_rejection_feedback`, `context_scopes_sections_to_task_epic_and_project`, `context_sibling_claims_are_effective_active_claims_on_other_tasks` (same-epic, clock-expired claims excluded without a sweep), `context_unknown_or_cross_project_task_is_not_found`, `context_remains_readable_on_archived_scope`. Explicit truncation with continuation links: `context_truncates_documents_with_exact_continuation_paths`, `context_truncation_drops_lower_priority_sections_first` (fixed priority: sessions → reviews → submissions → documents → sibling_claims → dependencies; first non-fitting item closes its section and all lower ones), `context_count_caps_set_truncated_with_continuations`, `context_mandatory_plus_plan_over_budget_is_context_too_large` (65,536 four-byte chars; exact getDocumentRevision continuation; `code() == "context_too_large"`), `context_minimal_bundle_is_not_truncated`. Coherent event groups / none on failure: `events_cover_creation_acceptance_and_update_transitions`, `events_cover_claim_report_and_review_transitions`, `events_cover_release_block_cancel_waive_and_completion_transitions`, `events_cover_dependency_document_and_selection_transitions` (plan/04 rows driven through public commands; `assert_group` checks shared command metadata, (kind rank, resource UUID) ordering, one bump per resource; no-op plan re-select emits nothing), `failed_transaction_appends_no_events` (`AFTER_EVENT_INSERT` failpoint; prior committed events survive; retry succeeds), `rejected_commands_leave_history_unchanged`. Stable IDs: `event_ids_survive_restart_and_resume_monotonically` (pool close + reopen on the same file), `idempotent_replay_appends_no_duplicate_events`, `history_and_replay_rereads_return_identical_ids`. History/replay bounds and negatives: `history_lists_newest_first_and_paginates`, `history_resource_filter_binds_cursor_fingerprint`, `history_unknown_project_is_not_found_before_cursor_validation`, `history_cross_project_resource_filter_leaks_nothing`, `replay_returns_ascending_project_events_after_cursor` (cross-project gaps valid), `replay_cursor_ahead_requires_resync`, `replay_cursor_older_than_seven_days_requires_resync` (TestClock + 8 days), `replay_backlog_over_limit_requires_resync` (1,000 replays in full; 1,001 resyncs), `concurrent_commands_produce_disjoint_coherent_event_groups` (two pools, one file, `tokio::join!`).

Design decisions recorded: listHistory uses the signed cursor codec with an i64 tiebreaker (empty sort column; fingerprint `project=…&resource=…`), while replay takes a plain `EventId` cursor matching plan/08's decimal wire cursor; continuation paths are exact collection paths without cursors (per-process HMAC keys do not survive restart); reviews and submissions share the `listSubmissions?task_id=` continuation (one review max per submission); sibling claims are effective-active (`status='active' AND expires_at > now`) claims on other tasks of the same epic; dependencies are epic links touching this epic plus task links touching this task; archived scope stays readable per plan/03; replay checks run cursor-ahead → window-expired (project-scoped O(1) probe on `events_project`) → bounded backlog count → ascending fetch, all in one read transaction with `now` from the store clock.

Temporary interfaces and deviations for step 015: `ContextBundle` carries `selected_plan_revision_ids`/`accepted_plan_submission_ids` as wire-Task conversion inputs read in the same snapshot (`TASK_COLUMNS` omits them); `session_from_row` is private to `queries/context.rs` and is the seam for `listTaskSessions`; the 256 KiB budget is measured over core-built wire-shaped JSON for Project/Goal/Epic/Task with a fixed empty-eligibility skeleton — eligibility is actor-relative and computed at 015 conversion, so 015 must keep the field sets identical and account for eligibility `reasons`/`allowed_actions` growth; `context_too_large` joins `graph_too_large` as a plan/05 code absent from plan/07's API-02 table; observed contract tension: wire `Event.resource_revision` has `minimum: 1` but `session.recorded`/`review.recorded` rows persist `0` (emission shape since steps 010/012) — the 015 wire mapping or a contract revision must resolve it; `ReplayOutcome::ResyncRequired` maps to the SSE `resync_required` frame and `event_high_water` is the no-cursor starting point.

Review round (same branch, post-handoff commit): the pre-merge code review surfaced and fixed — context documents now order by latest-revision recency (`dr.created_at DESC, dr.id DESC`), not document creation time, so truncation keeps the newest content per plan/05 (regression: revising an older project doc moves it to the front in `context_scopes_sections_to_task_epic_and_project`); the documents query became two-phase — a metadata pass (`LENGTH(dr.body)`, characters lower-bound serialized bytes) trims candidates to roughly one budget of body I/O before fetching full revisions, instead of eagerly materializing up to 101 full bodies (~26 MiB worst case) for a 256 KiB response; the replay window check reads only the single oldest row after the cursor (`ORDER BY id ASC LIMIT 1`) and compares in Rust, making it genuinely O(1) instead of a full-backlog scan when nothing is expired; the global high-water SQL is one shared `history::high_water(conn)` helper used by `Store::event_high_water` and `replay_events` (015 can call it inside a snapshot transaction); the unreachable `ContextTooLarge` fallback continuation (`{prefix}/documents`) became a `StorageError::Corrupt` — mandatory entity limits sum well below the budget, so overflow without a selected plan is an integrity failure, not a routable condition; the measurement/fill layer split into `src/queries/context/measure.rs` (wire-shaped mandatory JSON, `BudgetFill`, `CONTEXT_BUDGET_BYTES` re-exported) keeping `queries/context.rs` on query assembly per the module map; the acceptance suite split into `tests/v1_context_and_history/{main,fixtures,context_assembly,event_groups,history_replay}.rs` — the cargo test target name `v1_context_and_history` is unchanged. All gates re-run green after the round: fmt, clippy `-D warnings`, 654 workspace tests, coverage unit 97.1%/integration 93.1%/union 97.0%, `plan/validate.py`.

Second review round (same branch, commit `9bd95e1`): a pre-merge pass re-read the full diff against main and the contracts — one latent defect fixed: `context_too_large` was absent from `shepherd-server`'s `problem_for` code→status mapping (`src/middleware/problem.rs`), so it fell into the catch-all 500 arm; it now maps to 422 alongside `graph_too_large` with a case in `problem_for_maps_every_domain_code_to_its_plan07_status` (the context endpoint does not exist until 015, so no wire behavior changed yet). Verified clean: SQL parameter binding including the three `AssertSqlSafe` interpolation sites (compile-time constants only), cursor MAC cross-shape rejection, row-mapper column lists against the STRICT schema, budget accounting (per-item `len+1`, continuation-path reserve `Σ(len+3)` ≥ wire delta, skeleton brackets pre-counted), contract caps/priority/wire field sets against `openapi.yaml`, and index coverage for every new query. Observation recorded, no change: the context documents candidate scan is O(project-owned + this-epic-owned documents) with a `LENGTH()` per row before the `LIMIT 101` — bounded by the project's own data and accepted for v1 scale; revisit only if projects accumulate very large document counts. Gates re-run green after the fix: fmt, clippy `-D warnings`, all 21 workspace suites, `plan/validate.py`.

Next eligible step: [014 — portability](014-portability.md) (requires 013); step 015 needs 013 and 014.
