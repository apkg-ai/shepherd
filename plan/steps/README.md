# Implementation execution rules

Use one step per implementation session unless its handoff explicitly identifies a smaller subtask. Start with an eligible step whose prerequisites are complete. Read linked specifications before code. Implement its behavior and tests, run checks, fill the handoff, then mark it complete. No agent needs the original conversation.

All steps are initially not started. The generated manifest records prerequisites only, not runtime status. Independent branches after REST cutover are UI (016–020), clients (021–024), and operations (025); they join for packaging and acceptance. Do not edit the same files concurrently without explicit coordination.

## Intermediate compilation rule

Steps 002–014 develop new core under v1/ beside MVP core so existing server/tests still compile. Use v1::Store only in new tests; no preview endpoints. Step 015 promotes the complete contract, moves v1 modules to final locations, removes MVP core imports/routes and replaces legacy semantic fixtures. This is an intentional backend integration boundary. Frontend steps 016–020 regenerate once and keep unfinished routes unavailable; use MSW for screens not yet linked. Step 020 removes the route switch. No production release of intermediate work is required.

The dependency tree remains green at each boundary; don't add public NotImplemented endpoints. Existing MVP tests that encode obsolete behavior are replaced with mapped v1 tests at cutover, not retained as contradictory compatibility requirements. Do not disable unrelated tests to hide regressions.

## Ordered step index

| Step | Outcome | Requires |
|---|---|---|
| [001](001-contract-baseline.md) | Contract baseline and isolated generation | — |
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
| [015](015-api-cutover.md) | REST cutover and resumable SSE | 013, 014 |
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
| [028](028-cutover-and-release-docs.md) | Documentation cutover and release readiness | 027 |
