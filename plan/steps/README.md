# Implementation execution rules

Use one step per implementation session unless its handoff explicitly identifies a smaller subtask. Start with an eligible step whose prerequisites are complete. Read linked specifications before code. Implement its behavior and tests, run checks, fill the handoff, then mark it complete. No agent needs the original conversation.

All steps are initially not started. The generated manifest records prerequisites only, not runtime status. Independent branches after REST integration are UI (016–020), clients (021–024), and operations (025); they join for packaging and acceptance. Do not edit the same files concurrently without explicit coordination.

GitHub issue numbers are recorded in [github-issues.json](github-issues.json). Issue #12 is the umbrella roadmap; all implementation issues belong to the `v1 — stable agentic SDLC` milestone.

## Green-scaffold rule

Step 000 removes the MVP product implementation but preserves the compiling workspace, dependency pins, CI/security jobs, generators, test runners, minimal health server and accessible React shell. It must not touch user data. Later steps implement directly in final module paths; no `v1/` namespace, legacy source tree, preview endpoint, compatibility layer or route switch is allowed. Step 015 replaces the health-only scaffold contract with the complete implemented REST/SSE surface. Frontend steps 016–020 add only routes whose behavior is complete; use MSW for component isolation. No production release of intermediate work is required.

The dependency tree remains green at each boundary; do not add public NotImplemented endpoints. Step 000 removes tests that solely encode deleted MVP behavior while retaining applicable infrastructure, security and accessibility assertions. Subsequent tests describe stable-v1 behavior only. Do not disable a quality category to hide regressions.

## Ordered step index

| Step | Outcome | Requires |
|---|---|---|
| [000](000-scaffold-reset.md) | Reset to a green v1 scaffold | — |
| [001](001-contract-baseline.md) | Contract baseline and generation scaffolding | 000 |
| [002](002-storage-foundation.md) | Fresh database and command infrastructure | 001 |
| [003](003-projects-and-goals.md) | Project and goal ownership | 002 |
| [004](004-epics-and-tasks.md) | Separate epic and task resources | 003 |
| [005](005-dependencies-and-eligibility.md) | Scoped dependencies and pure eligibility | 004 |
| [006](006-completion-and-blocking.md) | Epic completion, block, cancellation and archive | 005 |
| [007](007-identity-and-permissions.md) | Local owner and agent credentials | 006 |
| [008](008-proposals-and-policy.md) | Proposal and policy configuration | 007 |
| [009](009-phase-claims.md) | Planning and execution leases | 008 |
| [010](010-reports-and-retries.md) | Atomic reports and failure recovery | 009 |
| [011](011-documents-and-submissions.md) | Versioned documents and plan selection | 010 |
| [012](012-reviews.md) | Human and independent agent review | 011 |
| [013](013-context-and-history.md) | Context assembly and durable history | 012 |
| [014](014-portability.md) | Snapshot export and full backup/restore | 013 |
| [015](015-rest-integration.md) | REST integration and resumable SSE | 013, 014 |
| [016](016-ui-shell-and-hierarchy.md) | Browser session and hierarchy navigation | 015 |
| [017](017-ui-task-details.md) | Task and epic forms, phases and actions | 016 |
| [018](018-ui-graphs.md) | Navigable scoped dependency graphs | 017 |
| [019](019-ui-documents-and-reviews.md) | Document editor and review workflows | 017 |
| [020](020-ui-live-settings.md) | Settings, identities and real-time integration | 018, 019 |
| [021](021-shared-rest-client.md) | Shared Rust REST client | 015 |
| [022](022-cli.md) | Complete CLI adapter | 021 |
| [023](023-mcp.md) | MCP stdio adapter | 021 |
| [024](024-agent-guide.md) | Runnable agent handoff guide | 022, 023 |
| [025](025-diagnostics.md) | Integrity diagnostics and graceful shutdown | 015, 014 |
| [026](026-release-packaging.md) | macOS and Linux packaging | 020, 022, 023, 025 |
| [027](027-acceptance-and-performance.md) | Cross-interface and performance acceptance | 024, 026 |
| [028](028-release-docs.md) | Documentation and release readiness | 027 |
