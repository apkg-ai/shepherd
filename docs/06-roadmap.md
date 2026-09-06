# 06 — Roadmap

> v1 is built in **nine sessions of ~2–3 hours** — deliberately shepherd-sized units of work. Every
> session ends **CI-green and shippable**; no session merges red. Each session is a GitHub issue on the
> **v1 milestone**; follow-ups are roadmap issues. Sessions are sequential unless noted.

## v1 sessions

| # | Session | Delivers | Done when |
|---|---|---|---|
| [S1](https://github.com/apkg-ai/shepherd/issues/4) | **Scaffolding & CI backbone** | Cargo workspace (`shepherd-core` lib + `shepherd-server` bin), `ui/` Vite app, `openapi/` placeholder passing Spectral, full CI skeleton (lint/test/smoke stages), `/health`. Closes the S1 decisions: TS formatter, Rust error/config crates, default port. | CI green; `cargo run` serves `/health`; `npm run dev` shows a shell page |
| [S2](https://github.com/apkg-ai/shepherd/issues/5) | **OpenAPI contract v1 + agent guide** | Complete `openapi/shepherd.yaml` per the [03](03-api.md) rules — every v1 feature, SSE catalog, error model, identity shapes, export document. Start/end graph representation decided. `docs/agent-guide.md`: how any harness drives the loop via REST. | Spectral green; spec covers 100% of [00](00-scope.md) v1 scope; guide walks the full agent loop |
| [S3](https://github.com/apkg-ai/shepherd/issues/6) | **Domain core & storage** | `sqlx` migrations for all [02](02-domain-model.md) entities; lifecycle state machine, DAG ops (cycle rejection), lease logic, bundle assembly in `shepherd-core`. Unit + property (`proptest` on all six invariants) + migration tests. | `cargo test` green incl. property suites; no HTTP code touched |
| [S4](https://github.com/apkg-ai/shepherd/issues/7) | **REST: projects, tasks, relations** | Handlers for project CRUD/settings, task CRUD + propose/approve/reject, relations with cycle rejection, `ready` derivation. Dynamic contract conformance wired into integration tests. | All endpoints conform to spec; integration + contract tests green |
| [S5](https://github.com/apkg-ai/shepherd/issues/8) | **REST: the agent loop** | `next-task`, claim/renew/release with TTL expiry, context bundle, session reporting (incl. failure → `ready` + attempt history), knowledge CRUD, export/import. Concurrency test: N parallel claims → one winner. | Full agent loop executable via `curl` alone, following `docs/agent-guide.md` |
| [S6](https://github.com/apkg-ai/shepherd/issues/9) | **Events & SSE** | Typed domain events from `shepherd-core` (broadcast), `/events` SSE endpoint with project filter, emission integration tests for the whole catalog. | Every mutating endpoint provably emits its event; SSE consumable via `curl` |
| [S7](https://github.com/apkg-ai/shepherd/issues/10) | **UI: shell, registry, detail, review** | Spec-derived API client; project registry (+ settings, export/import); task list/detail (metadata, relations, sessions timeline, attempt history, knowledge); review queue approve/reject; create/edit forms. Component tests. | Full CRUD + review flows usable in the browser against a real server |
| [S8](https://github.com/apkg-ai/shepherd/issues/11) | **UI: graph lenses & liveness** | Graph library decision recorded; decomposition-tree and dependency-flow lenses with the [04](04-ui.md) status language; SSE live updates; project-knowledge screen. | Both lenses render a seeded project and update live while an agent works |
| [S9](https://github.com/apkg-ai/shepherd/issues/12) | **Hardening & acceptance** | Playwright E2E of the primary flow; `cargo-fuzz` targets (metadata, mutation payloads, params, import docs) wired short/nightly; release smoke. **Acceptance dogfood**: real project, 15+ tasks / 3+ types / both relations, 2 Claude Code sessions via REST, session-2 knowledge reuse evidenced, start→end review in the browser. | [00](00-scope.md) acceptance criteria pass with recorded evidence; v1 tagged |

Dependencies: S1 → S2 → S3 → S4 → S5 → S6; S7 needs S4–S6; S8 needs S7; S9 needs all.

## Follow-ups ledger (post-v1)

Roadmap issues (larger items) and recorded deferrals (smaller ones — promoted to issues when scheduled):

| # | Item | Status |
|---|---|---|
| [F1](https://github.com/apkg-ai/shepherd/issues/13) | **CLI** — thin REST client (`shepherd task next`, `claim`, `report`, …), no business logic | roadmap issue |
| [F2](https://github.com/apkg-ai/shepherd/issues/14) | **MCP server layer** over the same REST semantics | roadmap issue |
| [F3](https://github.com/apkg-ai/shepherd/issues/15) | **Native app (Tauri)** — embed `shepherd-core`, reuse `ui/` | roadmap issue |
| [F4](https://github.com/apkg-ai/shepherd/issues/16) | **Token auth** for remote reachability (identity model is the hook) | roadmap issue |
| [F5](https://github.com/apkg-ai/shepherd/issues/17) | **Prebuilt binaries** + release pipeline (and npm wrapper alongside F1) | roadmap issue |
| [F6](https://github.com/apkg-ai/shepherd/issues/18) | **Load/envelope test** — low-thousands of tasks stay responsive | roadmap issue |
| F7 | Custom task-type schemas with validation | ledger |
| F8 | Notifications | ledger |
| F9 | Visual regression + accessibility (axe) checks | ledger |
| F10 | Import merge semantics (v1 import = new project only) | ledger |
| F11 | SSE replay / `Last-Event-ID` (v1: refetch on reconnect) | ledger |
