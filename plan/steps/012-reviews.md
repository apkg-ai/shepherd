# 012 — Human and independent agent review

Status: implemented on branch `v1-012-reviews` (PR pending). Requirements: REVIEW-01 REVIEW-02.

## Objective and prerequisites

Deliver human and independent agent review. Required completed steps: [011](011-documents-and-submissions.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/commands/reviews.rs`
- `core/shepherd-core/src/workflow/task.rs`
- `core/shepherd-core/src/commands/claims.rs`

Tests: `core/shepherd-core/tests/v1_reviews.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Implement pending submission queue, review claims, review decision and withdrawal commands. Pin policy and producer when submission is created. Human reviewer has no claim; agent reviewer needs distinct principal and valid review lease. Finish approve/reject transitions and completion cascades.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Producer cannot agent-review own output using another session. Agent token cannot approve human gate. Rejected plan returns planning; rejected work returns execution with plan retained. Withdrawal revokes review claim; late review fails. None policy creates accepted submission without fake human Review.

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

Branch: `v1-012-reviews`. Commits: `0e9466d` (review model vocabulary + error variants), `9cbb6f3` (review claim acquisition + permanent report guards), `816a57b` (review decisions with approve/reject transitions), `3e19c30` (submission withdrawal), `84f2cc9` (submission/review read commands), `fac3252` (revoke_agent claim closure), `258e443` (expiry/blocking/replay scenarios), `af10b9e` (in-module unit tests for the coverage gate).

Files changed: `core/shepherd-core/src/model/review.rs` (new: SubmissionKind/SubmissionStatus/ReviewDecision enums, wire `Submission` — omits persisted-only `withdraw_reason` — immutable `Review`, `ReviewInput`), `src/error.rs` (new variants `ReviewerPolicy` → `reviewer_policy` 403, `ProducerCannotReview` → `producer_cannot_review` 403, `SubmissionSuperseded` → `submission_superseded` 409), `src/commands/reviews.rs` (new: `review_submission`, `withdraw_submission`, shared `review_gate_error`, in-module unit suite), `src/commands/claims.rs` (review-phase claims implemented: `ReviewAgentPolicy` capability gate, `can_review` eligibility, pending-submission pin, no task bump — `claim.changed` only per plan/04), `src/commands/reports.rs` (scaffold guards now permanent `invalid_state`: review claims never report, sessions CHECK only stores plan|execute), `src/queries/submissions.rs` (new: shared `submission_row` loader/mapper, `list_submissions` newest-first descending keyset with task/status/kind/policy filters, `get_submission`, `list_submission_reviews`), `src/commands/identity/agents.rs` + `commands/mod.rs` (`ClaimScope::Actor`, `PendingEvent::review` rank 10, revocation closes claims), `src/model/mod.rs`/`model/content.rs`/`queries/mod.rs` (wiring; `serde_revision` widened to `pub(super)`), `tests/v1_reviews.rs` (new, 35 named tests), `tests/v1_phase_claims.rs` (scaffold-guard test became `review_claim_shape_is_validated`).

Test commands and results (run from `core/`): `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` all suites pass (341 lib tests incl. 10 review command/read unit tests; 35 v1_reviews integration tests; 601 workspace total); `cargo test --workspace --doc` pass; coverage via `cargo llvm-cov` + `node scripts/coverage-report.ts --dir core/coverage --report core`: unit 96.9% (gate ≥ 95), integration 93.1% (gate ≥ 70), union 96.8% (gate ≥ 92); `python3 plan/validate.py` PASS. No reproduced limitation; the served contract in `openapi/shepherd.yaml` is untouched — review operations are core-only until step 015 wires HTTP (If-Match/Idempotency-Key/X-Lease-Token header mapping; wire `ReasonInput` maps to the `reason: String` parameter).

Acceptance sentence → regression test: producer/other-session → `producer_cannot_agent_review_own_output_using_another_session` (fresh claim = fresh session; a separately registered agent succeeds, D06); agent on human gate → `agent_token_cannot_approve_human_gate` (plus `owner_cannot_decide_agent_policy_review` for the hard agent gate, D15); rejected plan → `rejected_plan_returns_planning_and_clears_selection_and_acceptance` — **discharges the 010/011 obligation: rejection clears both `selected_plan_revision_id` and `accepted_plan_submission_id`**; rejected work → `rejected_work_returns_execution_with_plan_retained`; withdrawal → `withdrawal_revokes_review_claim_and_late_review_fails`; none policy → `none_policy_accepts_submission_without_fake_review` (zero reviews rows, zero review.recorded events). Adversarial coverage: stale If-Match + idempotent replay on decide and withdraw, two-pool races (double review claim; decide-vs-withdraw), TTL expiry via TestClock with queue recovery, block revokes the claim while the submission stays pending, superseded submission ids, blank/oversize reasons, with persisted-state rollback asserts (`mutation_snapshot`).

Design decisions recorded: review claims close as `released` on decision, `revoked` on withdraw/block/cancel/actor revocation, `expired` on TTL — never `reported`; `review.recorded` is immutable (resource_revision 0, rank 10, affected `[submission, task]`); a failed decision rolls back whole including the reconcile sweep, so a stale claim row reads `active` until the next successful command sweeps it (asserted in `expired_review_claim_fails_late_decision_and_frees_the_queue`); missing lease token on an agent decision maps to `lease_invalid` (409), not `precondition_required` — 428 stays revision-only; 403 (`reviewer_policy`/`producer_cannot_review`) wins over 409 `not_eligible` per plan/04 "human policy enforcement always returns 403"; the review-report guard is permanent `invalid_state` (pre-existing API-02 deviation, re-noted).

Boundary exception (owner-approved): `commands/identity/agents.rs` is outside this step's file list, but `revoke_agent` violated plan/03/plan/12 ("revoking an actor revokes all its active claims") — fixed in the dedicated commit `fac3252` via `ClaimScope::Actor` with per-project `claim.changed` events; regression `revoking_reviewer_closes_claim_and_frees_the_review` lives in `tests/v1_reviews.rs` (not the identity suite) because the full public review flow exists there.

Remaining `testing::seed_claim`/`seed_submission` call sites (kept deliberately): `tests/v1_completion_and_blocking.rs` and `tests/v1_proposals_and_policy.rs` seed claims/submissions to isolate active-work guards with exact revision/event-count assertions — driving the public claim flow there would bump revisions and add events, weakening those assertions; the review flow itself is exercised exclusively through public commands in `tests/v1_reviews.rs` and the reviews in-module suite. `Store::report_failpoint` and `complete_task_for_test` remain test-only drivers; `select_task_plan` idempotency remains the step-015 obligation.

Next eligible step: [013 — context and history](013-context-and-history.md), which consumes the review events and immutable records added here.
