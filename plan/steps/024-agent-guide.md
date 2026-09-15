# 024 — Runnable agent handoff guide

Status: not started. Requirements: API-01 CONTENT-01.

## Objective and prerequisites

Deliver runnable agent handoff guide. Required completed steps: [022](022-cli.md), [023](023-mcp.md)

Read [execution rules](README.md) first, then:

- [00-product-scope.md](../00-product-scope.md)
- [11-agent-guide.md](../11-agent-guide.md)
- [14-test-strategy.md](../14-test-strategy.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `docs/agent-guide.md`
- `scripts/v1-agent-handoff-e2e.sh`
- `tests/hurl/10-v1-agent-handoff.hurl`
- `tests/v1-agent-workflow.py`

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the current scaffold and targeted tests. Identify the reusable plumbing and final modules owned by this step; do not restore removed MVP product behavior.
2. Promote plan/11-agent-guide.md and validated examples into stable docs/agent-guide.md while preserving user untracked files. Turn example operation sequences into live Hurl/CLI/MCP acceptance scenarios with captured IDs and revisions. Add `scripts/v1-agent-handoff-e2e.sh`: it starts the daemon with an isolated fresh data directory, bootstraps owner plus distinct planner/reviewer/executor credential files, runs `tests/hurl/10-v1-agent-handoff.hurl` and `tests/v1-agent-workflow.py` against that daemon, and always tears the process and data directory down. Explain polling/renewal, unknown response retry and handoff content.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Fresh independent agent can follow instructions without old conversation. All example operations exist. Planner/reviewer/executor use distinct credentials; guide never tells agent to impersonate owner or bypass human gates.

The wrapper proves both live workflows against one freshly bootstrapped daemon and leaves no daemon or data directory behind on success or failure.

For each sentence above create a named regression test with setup → action → expected status/error → persisted state checks. Mutation failures must leave resource revision/history unchanged except an independently committed prior command. Use file-backed SQLite and independent pools for concurrency, controllable clock for TTL; do not test only a mocked helper that mirrors implementation. For UI, cover loading/empty/error plus keyboard interaction for new controls, using MSW for component tests and real server for critical E2E.

## Excluded work

Do not implement subsequent steps or change confirmed product decisions. No external-agent spawning, recursive hierarchy, cross-goal dependency, server-wide auth bypass or MVP data converter. Only add dependencies pinned in [dependency choices](../dependencies.md). Never reset or delete the user's existing database to make tests pass.

## Verification

```sh
# Repository root.
python3 plan/validate.py
node plan/validate-contracts.cjs
scripts/v1-agent-handoff-e2e.sh
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

Fill when implementing, not during planning: commit/branch; files changed; test commands and results; any reproduced limitation; temporary interfaces; next eligible step IDs. If an acceptance criterion cannot pass, leave this step incomplete and explain the concrete blocker. Do not mark complete for code that merely compiles.
