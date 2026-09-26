# 005 — Scoped dependencies and pure eligibility

Status: implemented on branch `v1-005-dependencies-and-eligibility` (PR pending). Requirements: DEP-01 FLOW-02.

## Objective and prerequisites

Deliver scoped dependencies and pure eligibility. Required completed steps: [004](004-epics-and-tasks.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/workflow/eligibility.rs`
- `core/shepherd-core/src/commands/dependencies.rs`
- `core/shepherd-core/src/queries/{graph,work}.rs`
- `core/shepherd-core/src/dag.rs`

Tests: `core/shepherd-core/tests/v1_dependencies_and_eligibility.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Reuse dag cycle traversal with epic/task IDs. Implement two dependency tables behind one typed command. Add complete GateReason enum and evaluate_task/evaluate_epic functions using preloaded snapshots. Return actor-dependent allowed_actions. Implement listWork and scoped graph queries with bounded node/edge output.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Cross-goal epic link and cross-epic task link fail; valid branch/join passes. Candidate is eligible with zero prerequisites; waits until every prerequisite done. Early planning ignores dependency waits but not proposal/manual blocks. Cycle race yields one accepted edge at most.

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

- [ ] Referenced requirements and every acceptance sentence implemented.
- [ ] Schemas, permissions, transitions and clients remain consistent.
- [ ] Success and rejection behavior verified at the public boundary.
- [ ] Required commands pass; material environmental limitation recorded accurately.
- [ ] Temporary code and next-step dependencies documented.
- [ ] Handoff below completed; next eligible step linked.

## Implementation handoff record

Branch `v1-005-dependencies-and-eligibility` (PR pending).

### Files changed

- New: `core/shepherd-core/src/{dag.rs, workflow/eligibility.rs, commands/dependencies.rs, queries/graph.rs, queries/work.rs}`, `core/shepherd-core/tests/v1_dependencies_and_eligibility.rs`.
- Amended: `lib.rs` (`mod dag`), `workflow/mod.rs` (declare eligibility), `commands/mod.rs` (`mod dependencies`, `PendingEvent::dependency` — first non-empty `affected_ids`), `queries/mod.rs` (declarations + re-exports), `queries/hierarchy.rs` (pub(crate) widenings only: `TASK_COLUMNS`/`EPIC_COLUMNS`/`parse_review_policy`/`CountScope`/`task_counts_by`/`filter_token`), `model/mod.rs` (`DependencyId`, `DependencyLevel`, `Dependency`, `DependencyCreate`, `EntityStatus` + From impls), `error.rs` (`ActiveWork`/`DependencyCycle`/`ScopeMismatch`/`GraphTooLarge` + `code()` arms).
- No new crate dependencies and no migration: `epic_dependencies`/`task_dependencies` and the `dependency.changed` event type ship in the baseline schema.

### What was built

`dag::would_create_cycle` recovered from git history (`5619c7c`), reduced to the cycle traversal only (single-parent/ancestor/downstream/GraphRole stayed removed) and made generic over the id type for epic/task reuse; commands load the scoped edge list inside the serialized write transaction. `create_dependency` is the one typed command over both tables (`DependencyCreate::{Epic,Task}`): membership on both endpoints (404) → archived chain → terminal dependent (done prerequisite allowed and immediately satisfied; cancelled stays unmet) → active-work on the dependent (unexpired claim or pending submission; epic level checks every descendant task) → agent-adds-to-accepted-dependent guard → non-self → same-goal/same-epic scope (`scope_mismatch`) → duplicate pair → acyclicity (`dependency_cycle`) → insert (revision 1) → both endpoint revisions bump (plan/03 "affected tasks/epics"; descendants untouched — derived eligibility needs no stored revision) → `dependency.changed` with `affected_ids=[dependent, prerequisite, scope]` plus both endpoint `.changed` events. `delete_dependency` is human-only with If-Match on the dependency revision and the same archived/terminal/active-work guards. `workflow::eligibility`: complete 15-code `GateCode` in contract order, pure `evaluate_task`/`evaluate_epic` over `TaskSnapshot`/`EpicSnapshot` per the plan/04 pseudocode (archived-before-terminal early exit — valid archived records are always terminal, so the archived gate must fire first to keep them distinguishable in graphs; strict `== Done` prerequisites; claim expiry `<= now`; plan_ok = selected revision ∈ accepted submission's document_revision_ids; review policy + producer exclusion), deterministic reasons with blocking resource ids (prerequisite reasons capped so the emitted list stays inside the openapi `Eligibility.reasons` maxItems of 1000 — nothing caps stored links, and the flags still consider every prerequisite), and `allowed_actions` as the full REST-operationId vocabulary ∩ plan/12 capability matrix; batch snapshot loaders with constant query counts (no per-node queries). `queries::work::list_work`: eligible-only, caller+phase scoped, oldest first; SQL prefilter (superset) → batch snapshots → evaluator filter → refill loop keyset over candidates while the emitted cursor anchors the last returned item. `queries::graph`: `get_goal_graph`/`get_epic_graph` (bounds first — >2000 nodes/>4000 edges → `graph_too_large`, all-or-nothing; archived nodes included with their `archived` gate; counts via the batched GROUP BY; eligibility via loaders + evaluators with the query clock) and `list_dependencies` (UNION ALL over both tables, level/scope_id filters, shared keyset cursors).

### Test commands and results (Rust 1.98.1)

- From `core/`: `cargo fmt --check` — clean; `cargo test --workspace` — 292 green (exit code checked directly, never piped); `cargo clippy --all-targets -- -D warnings` — clean; `cargo test --workspace --doc` — clean.
- Coverage via the CI llvm-cov commands + `node scripts/coverage-report.ts --dir core/coverage --report core`: Unit 97.9% (≥95) ✅, Integration 100.0% (≥70) ✅, union 97.9% (≥92) ✅.
- `python3 plan/validate.py` — PASS (29 step DAG, 70 operations), rerun after this document update.
- Every acceptance sentence has a named regression test in `tests/v1_dependencies_and_eligibility.rs`: `cross_goal_epic_link_is_scope_mismatch`, `cross_epic_task_link_is_scope_mismatch`, `valid_branch_and_join_passes` (diamond at both levels; revision bumps and `affected_ids` asserted), `candidate_with_zero_prerequisites_is_eligible`, `candidate_waits_until_every_prerequisite_done` (incl. cancelled-and-waived prerequisite staying unmet), `early_planning_ignores_dependency_waits_but_not_proposal_or_manual_blocks`, `cycle_race_yields_at_most_one_accepted_edge` (two independent pools on one file-backed database; exactly one edge, one `dependency.changed`, loser sees `dependency_cycle`), `failed_removal_leaves_revision_and_history_unchanged`. Persisted-state checks use an independent read-only connection.
- In-module suites: 29 pure evaluator tests (contract reason order, expiry boundary `expires_at == now`, strict-done, plan gate, review policies, action matrices, reasons contract bound), 14 command tests (guards, agent rules, expired claims unblock, epic descendant checks, terminal owning epic), 7 list_work tests (actor-bound cursors, keyset across ineligible rows, review caller-dependence, clock-flipped claim expiry), 6 graph tests (scoping, archived nodes, 2001-node all-or-nothing bound, union paging).

### Limitations and temporary interfaces

- `ClaimSnapshot.phase` and `SubmissionSnapshot.kind` carry raw stored strings ('plan'/'execute'/'review', 'plan'/'work'); the typed claim/submission models land in steps 009/011, when `queries::work::WorkPhase` should unify with claim phases.
- Claims, submissions, blocks, completion and archive states are seeded via direct SQL in tests with step-ownership comments (commands land in 006/009/010/011); the loaders read the baseline tables today.
- `graph_too_large` appears only in plan/05 (not plan/07 §API-02's 422 row) — wire mapping to be settled at step 015; likewise the agent-links-proposed guard returns `InvalidState` (`invalid_state`) pending a 015 decision.
- `allowed_actions` emits the full final operationId vocabulary now (decision: stable strings through 006–012); claimant-scoped extras (renew/release/report, agent `reviewSubmission`) slot in at 009 via an additional snapshot input without renaming.
- `list_work`'s refill loop can scan the whole prefiltered candidate set when few rows are eligible — accepted for local v1 scale.
- listWork cursors bind the caller's actor id into the filter fingerprint (deliberate deviation from other lists: review results are caller-dependent; cross-actor replay → `invalid_cursor`).
- Epic snapshots bake the descendant active-work aggregate with the load-time clock; task-level claim expiry stays raw for the pure evaluator.
- Decisions confirmed with the owner during planning: `delete_dependency` ships in 005; dependency mutations bump both endpoint revisions; `listDependencies` ships here; graphs include archived nodes; the agent accepted-work guard applies to the dependent only; `DomainError::ActiveWork` added now.

### Review follow-ups (local self-review, pre-PR)

A high-effort local review of `main...HEAD` produced eight findings; six fixed on the branch, two rejected with rationale:

- **Archived-goal reason misreport** [medium]: with only the goal archived, the `archived` gate's resource_id pointed at the (live) project because `TaskSnapshot` carried no goal id. Added `goal_id` to the snapshot; the reason now names the archived goal; the archived-chain test asserts the resource id for all four levels.
- **Agent guard ignored proposed epics** [medium]: `task_endpoint.proposed` reflected only the task's own status, so an agent could link a dependent task living under a proposed epic — looser than the eligibility definition of accepted (task AND epic not proposed). The endpoint now folds the owning epic's proposed state in; regression added.
- **selectTaskPlan advertised to non-claimants** [medium]: while a plan claim is active the button appeared for every actor; plan/04's carve-out is for the claimant. The action now requires `claim.actor_id == actor.id`; test asserts a bystander does not get it.
- **list_work scan chunk** [efficiency]: the refill loop chunked by `limit + 1`, so small pages over mostly-ineligible rows triggered many snapshot rounds. The scan chunk is now `max(limit + 1, 256)`, independent of the page limit (the cursor anchors the last returned item, not the scan position).
- **Scoped-edge mapping duplicated 4×** [simplification]: graph payloads, bounds check, listDependencies arm and the command's cycle-check edges each restated the level→parent mapping. Single `scope_parent`/`scoped_dependent_clause` source in `queries/graph.rs` now feeds all four.
- **Claim-expiry predicate re-inlined** [simplification]: the reason emission and actions filter re-tested `expires_at > now`; both now go through `claim_is_active`.
- Rejected — "multiple active claims collapse in the loader": impossible; the `one_active_claim_per_task` / `one_pending_submission_per_task` partial unique indexes guarantee at most one row per key (comments added citing the invariant).
- Rejected — "split eligibility.rs (evaluators vs loaders)": the plan/05 module map is authoritative for the final tree (`workflow/{mod,epic,task,eligibility,policy}.rs`) and `commands/hierarchy.rs` sets the size precedent; kept single-file with the pure/storage seam documented.

Post-fix results (self-review round): `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test --workspace --doc` clean; `cargo test --workspace` 292 green; coverage Unit 97.9% / Integration 100.0% / union 97.9%; `python3 plan/validate.py` PASS.

### Review follow-ups (CodeRabbit, on-PR)

Three inline findings on PR #86, all verified against the tree and fixed:

- **createDependency advertised under proposed epics** [functional]: `allowed_task_actions` excluded agents only for a proposed task, but `task_endpoint` (since the self-review fix above) rejects agents when the owning epic is proposed too — an agent seeing an `Open` task under a `Proposed` epic got a button that always fails. The agent branch now also requires `epic_status != Proposed`; owners keep the exception. Regression: `agents_cannot_link_dependents_under_proposed_epics`.
- **Terminal gate shadowed the archived gate** [minor]: plan/03 requires terminal before archive, so the terminal early-exit fired first and valid archived records could never report `archived` (the graph test only passed because its direct-SQL fixture archived a non-terminal epic, which the domain forbids). Both evaluators now check the archived chain first, matching the `guard_mutation` failure-precedence order. Regression: `archived_terminal_records_report_the_archived_gate`.
- **Prerequisite reasons could exceed the contract** [minor]: nothing caps stored links, so a dependent with >1000 unmet prerequisites emitted more reasons than the openapi `Eligibility.reasons` maxItems of 1000 — bounding accepted links cannot guarantee this (cross-level sums), so the evaluator caps emitted prerequisite reasons at `MAX_REASONS - 13` (13 = every non-prerequisite gate code, reserved so tail gates are never cut); the can-execute/can-plan flags still consider every prerequisite. Regression: `prerequisite_reasons_stay_within_the_contract_bound`.

Post-fix results: `cargo fmt --check` / `cargo clippy -p shepherd-core --all-targets -- -D warnings` clean; `cargo test -p shepherd-core` 285 green (184 lib + 101 integration).

### Review follow-ups (full codebase review, on-branch)

A five-track review (security, performance, step-005 correctness, steps 000–004 re-verification, issue/contract conformance) over the whole v1 tree; plan in `review/005-review-plan.md`, findings register in `review/005-compliance-review.md`. Confirmed and fixed:

- **update_task missing the active-work guard** [security, medium]: plan/03 requires "no active claim/pending review" for every task edit, and `updateTask` is only advertised unclaimed — but the command accepted edits under an active claim or pending submission. Added the guard (shared `has_active_work` helper moved to `commands/mod.rs`); regression `update_task_rejects_active_claims_and_pending_reviews` covers the claim case and the `expires_at == now` boundary.
- **Dependent under a terminal epic could gain dependencies** [security, low]: `task_endpoint.terminal` ignored the owning epic's status while the action requires `epic_live`. The endpoint now folds the epic's terminal state in (only the dependent is gated — terminal prerequisites stay allowed); regression `terminal_owning_epic_blocks_the_dependent_task`.
- **Endpoint events carried empty affected_ids** [conformance, medium]: plan/08 says "epic/task changes include owning goal/epic IDs". `PendingEvent::epic/task` now take the owning id; all call sites updated; dependency endpoint events carry `Endpoint.scope` (already the owning epic/goal).
- **blockTask/cancelTask ignored the owning epic** [correctness, low]: both now require `epic_live` like their siblings; a nonterminal task under a terminal epic advertises no mutations (asserted in the terminal-epic evaluator test).
- **Missing project-scope indexes** [performance, medium]: `epics(project_id,created_at,id)` and `task_dependencies`/`epic_dependencies` `(project_id,created_at,id)` added to the migration and `plan/contracts/schema.sql` (parity test enforces both).
- **list_work paid snapshot loads for blocked/claimed candidates** [performance, medium]: the SQL prefilter now also excludes `block_actor_id IS NOT NULL` and unexpired active claims (both are necessary conditions for every phase's eligibility; pending submissions stay in the superset because review eligibility needs them).
- **Domain-impossible fixtures** [tests, low]: the graph test archived a non-terminal epic (now completed first, per plan/03); the integration test flipped a cancelled+waived prerequisite back to done — replaced with the owner deleting the obsolete link via the public command; missing step-ownership comments added.
- **Step-004 handoff back-filled** [process, medium]: the step shipped merged with "Status: not started"; the record is now reconstructed from the squash commit and tree.

Rejected on evidence: a claimed missing `dependent_id` index (the `UNIQUE(dependent_id,prerequisite_id)` auto-index already serves those probes); a write-side edge cap (`graph_too_large` is plan/05's designed over-bound behavior); `pub` eligibility types (plan/05's final API seam). Full rationale in the review report.

Post-review results: `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 297 green; `python3 plan/validate.py` PASS; contract gates (smoke, operation freeze, v1 baseline) PASS; semgrep (p/rust, p/default) 0 findings; shellcheck clean.

Next eligible step: [006](006-completion-and-blocking.md).
