# MVP audit and replacement map

Inspected 2026-09-14. Findings are source inspection, not measured load-test results.

| Existing location | Current behavior | V1 treatment |
|---|---|---|
| core/shepherd-core/src/model.rs | Epic is TaskType; no mandatory epic_id; project boolean review_gate | Split types; explicit hierarchy and review policies |
| core/shepherd-core/src/lifecycle.rs | Task transitions include EpicAutoComplete | Replace with independent epic/task rules and pure eligibility |
| core/shepherd-core/src/store.rs | Mixed SQL, domain, events, tests; report releases claim and writes session/knowledge/status separately | Transaction-owned command modules; inject failures between writes |
| core/shepherd-core/src/dag.rs | Pure cycle and single-parent checks | Reuse cycle traversal; ownership becomes foreign keys |
| core/shepherd-core/src/lease.rs | TTL mechanics | Reuse bounded time calculations, enforce inside transactions |
| core/shepherd-core/src/bundle.rs | Ancestor/session context, no selected reviewed plan | Replace assembly with phase-specific explicit revisions |
| core/shepherd-core/src/event.rs | Memory broadcast only | Persist events, use broadcast as wakeup |
| core/shepherd-core/src/export.rs | Version 1 export, validated imports | Format 2, consistent read snapshot, active-state normalization |
| core/shepherd-server/src/lib.rs | Thin generated-trait implementation and SSE | Keep separation; resource handlers and durable replay |
| core/shepherd-server/src/middleware.rs | No real identity enforcement; fake rate headers; generator errors mostly 422 | Authenticate; truthful HTTP failures and local Origin/Host checks |
| core/shepherd-server/src/main.rs | Loopback, relative ui/dist, printed startup messages | Preserve local server; absolute asset resolution, shutdown, diagnostics |
| ui/src/router.tsx | Project/task routes, project graph landing | Goal/epic navigation and independent task detail |
| ui/src/screens/graph | React Flow selection/side panels | Preserve canvas behavior; replace decomposition-derived data |
| ui/src/lib/taskTree.ts | Task-to-task parent inference | Remove once goal/epic ownership adapter is live |
| ui/src/screens/review | Proposed/in_review task queries | Explicit proposal + plan/work submission queues |
| ui/src/api/client.ts, lib/events.ts | Central fetch/error handling; refetch on SSE reconnect | Add headers/session handling and replay/resync |
| ui/src/components and styles | Reusable accessible primitives and tokens | Preserve; extend badges for phase and eligibility |

Strengths: SQLite WAL/foreign keys, transactional claim and relation insertion, active-claim uniqueness, pure lifecycle/DAG helpers, contract generation, property tests, API/Hurl/Playwright tests, dependency/SAST CI. Preserve these; replace affected tests with stronger domain invariants rather than deleting coverage.

Migration-history editing and schema-string checks in store startup are MVP compatibility scaffolding, not a stable migration strategy. New baseline uses a separate path and explicit version identity. Current docs calling the MVP “v1” are historical; plan/ governs the new stable version. Do not edit user-owned untracked docs/agent-quickstart.md or example/ as incidental cleanup.

Installed evidence: Rust/Cargo 1.98.1, openapi-to-rust 0.16.0, Hurl 8.0.1. Node is installed in ~/.nvm/versions/node/v24.19.0/bin but absent from the default tool shell PATH. Read .nvmrc and activate Node before npm checks. scripts/regen-generated.sh invokes cargo fmt and therefore is not a read-only command. Rust generated output is gitignored; verify generation by compilation and wire conformance, not only git diff.
