# 008 — Proposal and policy configuration

Status: implemented on branch `v1-008-proposals-and-policy` (PR pending). Requirements: REVIEW-01 AUTH-01.

## Objective and prerequisites

Deliver proposal and policy configuration. Required completed steps: [007](007-identity-and-permissions.md)

Read [execution rules](README.md) first, then:

- [03-domain-model.md](../03-domain-model.md)
- [04-workflow-state-machines.md](../04-workflow-state-machines.md)
- [05-backend.md](../05-backend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [12-security-and-local-identity.md](../12-security-and-local-identity.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `core/shepherd-core/src/workflow/policy.rs`
- `core/shepherd-core/src/commands/hierarchy.rs`

Tests: `core/shepherd-core/tests/v1_proposals_and_policy.rs`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Finalize project settings/task override patch semantics and proposal acceptance. Add task type create/rename-label/archive commands. Define description/policy edits that invalidate selected-plan acceptance, using transition helpers rather than direct SQL status assignment. Separate proposal acceptance from review decisions.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Agent proposal gate defaults on and can be disabled by owner. Accepting epic leaves proposed tasks proposed. Policy downgrade fails for agents; human edit fails during active work. Archived type cannot be assigned but existing task still renders its label.

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

Branch `v1-008-proposals-and-policy` (PR pending).

### Files changed

- New: `core/shepherd-core/tests/v1_proposals_and_policy.rs` — the four acceptance sentences as named regressions at the public command boundary, plus the task-type archive negatives (one-way flag, double archive, archived project, agent forbidden).
- Amended: `core/shepherd-core/src/model/project.rs` (`TaskTypePatch.archived: Option<bool>` — the contract patch field), `core/shepherd-core/src/commands/hierarchy.rs` (`update_task_type` applies the archived flag in the same revision bump/event as a label rename; `Some(false)` is rejected because archive is one-way), `core/shepherd-core/src/commands/archive.rs` (the draft `archive_task_type` command was removed in review — see below), `core/shepherd-core/tests/v1_projects_and_goals.rs` and `core/shepherd-core/tests/v1_identity_and_permissions.rs` (existing `TaskTypePatch` literals gained `..Default::default()`).

### What was built

The step's behavior was mostly in place from steps 004–007 (proposal gate on create, policy snapshots with agent downgrade floors in `workflow/policy.rs`, accept-epic cascade that leaves proposed tasks proposed, task-type create/rename). This step closed the one missing command surface — task-type archiving — and pinned all four acceptance sentences as named regressions. Design note from review: the contract has no `archiveTaskType` operation; archiving rides `updateTaskType` via `TaskTypePatch.archived` (`plan/contracts/openapi.yaml`), so the draft separate command was folded into `update_task_type`. The event action stays `updateTaskType` per plan/08 (action = operationId or system.*), event type `task_type.changed`, empty reason — the contract has no `ReasonInput` for task types. Archived types stay assignable on neither create nor edit (`Validation` on `type_key`), existing tasks keep their `type_key`, and the registry row keeps key and label so reads still render it.

### Test commands and results

- From `core/`: `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --workspace` — 430 passed, 0 failed, exit code checked directly (never piped).
- Repository root: `python3 plan/validate.py` PASS (29 step DAG, 70 operations).
- Acceptance sentences → named tests: gate default/disabled — `proposal_gate_defaults_on_and_owner_disables` (persisted rows prove old proposed items are untouched after the gate flips); accept cascade — `accepting_epic_leaves_proposed_tasks_proposed` (task revision/status asserted unchanged); policy floors — `policy_downgrade_fails_for_agents_human_edit_fails_during_active_work` (create and update downgrades forbidden for agents, owner edit blocked by an active claim, event-count invariant proves failed commands leak no events); archived type — `archived_type_cannot_be_assigned_but_existing_task_renders_label` (plus `archived: Some(false)` rejection, double archive, archived project, agent forbidden).

### Limitations and temporary interfaces

- REST wiring for `updateTaskType` (and `listTaskTypes` with `include_archived`) lands in step 015; the generated wire `TaskTypePatch` already carries `archived` from the contract, so the core patch maps one-to-one.
- No temporary interfaces; no migration (the `archived` flag ships in the baseline `task_types` schema).

Next eligible step: [009](009-phase-claims.md).
