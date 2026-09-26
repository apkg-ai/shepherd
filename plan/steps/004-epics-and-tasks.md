# 004 — Separate epic and task resources

Status: implemented and merged (PR #85, squash commit f0722bb). Requirements: HIER-01 FLOW-01.

## Objective and prerequisites

Deliver separate epic and task resources. Required completed steps: [003](003-projects-and-goals.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/model/{epic,task}.rs`
- `core/shepherd-core/src/workflow/{mod,policy}.rs`
- `core/shepherd-core/src/commands/hierarchy.rs`

Tests: `core/shepherd-core/tests/v1_epics_and_tasks.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Add separate models, SQL row converters, create/get/list/update commands and policy snapshots. Initialize task phase from planning_required and proposal status from actor/project policy. Make ownership immutable and task types registry-backed. Implement resource revisions and terminal edit guard.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Task without epic fails; wrong-project epic fails. Epic is never a TaskType or claimable task. Changing defaults affects only future tasks. Agent cannot lower inherited requirements. No status patch is accepted.

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

Branch `v1-004-epics-and-tasks`, squash-merged as f0722bb (PR #85). This record was back-filled from the merge commit and tree during the step-005 full review; the step shipped before its handoff was written.

### Files changed

- New: `core/shepherd-core/src/model/{epic,task}.rs`, `core/shepherd-core/src/workflow/{mod,policy}.rs`, `core/shepherd-core/tests/v1_epics_and_tasks.rs`.
- Amended: `commands/hierarchy.rs` (epic/task create/get/list/update/accept commands), `queries/hierarchy.rs` (row converters, batched counts, scoping), `commands/mod.rs`, `error.rs`, `model/{mod,project}.rs`, `lib.rs`, `queries/mod.rs`.

### What was built

Separate `Epic`/`Task` models with distinct status enums (`resource_status!` macro) and the five-phase task workflow (planning → plan_review → execution → work_review → complete). Commands: create/get/list/update/accept for both resources, scoped membership (404 across projects), owner-only accept, immutable ownership, task-type registry validation (`^[a-z][a-z0-9_]{0,39}$`), policy snapshots from project defaults at creation, agent-cannot-lower policy enforcement (`enforce_agent_floor`, only when the patch touches policy fields), terminal-edit and archived-chain guards (including the epic terminal status on task mutations), phase reset on actual (not merely present) policy/description change with plan-acceptance clearing, and batched task-count aggregation with waived counting in one conditional-aggregation query. No status PATCH exists (accept is the only proposed → open transition).

### Test commands and results (Rust 1.98.1)

- From `core/`: `cargo fmt --check` — clean; `cargo clippy --all-targets -- -D warnings` — clean; `cargo test --workspace` — 214 green at merge (159 → 187 → 192 → 214 across the PR's fix cycles, per the squash-commit message); unit coverage raised from 80.6% to 93.4% (CI-measured) with the in-module suites added during review.
- Every acceptance sentence has a named regression test in `tests/v1_epics_and_tasks.rs`: `task_without_epic_fails`, `wrong_project_epic_fails`, `epic_is_never_a_task_type_or_claimable`, `changing_defaults_affects_only_future_tasks`, `agent_cannot_lower_inherited_requirements`, `no_status_patch_is_accepted` — all verified against the current tree by the step-005 review.

### Limitations and temporary interfaces

- Archive/cancel/complete commands land in step 006; archived/terminal states in tests are direct-SQL fixtures with step-ownership comments.
- The recompute cascade is a stub (full cascade logic lands with completion in 006).
- Cross-entity counts (`Goal.completed` derivation) read the batched GROUP BY; no per-node queries.

### Review follow-ups (PR review, pre-merge)

Per the squash-commit message: archived-scope gaps fixed (update/accept epic and task now check the owning goal/epic archived chain), failure precedence corrected (capability before revision in `update_task`), no-op phase reset fixed (value comparison instead of field presence), and the compliance review's PR-scoped findings addressed (SEC-01 boundary documentation, SEC-04 `CountScope` enum, SEC-05 validated type_key echo, PERF-02 single-query counts, QUAL-04 status macro, QUAL-05/06 cleanups). The full findings register lives in `review/004-compliance-review.md` (working tree, untracked by convention).

Next eligible step: [005](005-dependencies-and-eligibility.md).
