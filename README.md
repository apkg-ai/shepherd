# shepherd

Shepherd is a local-first visual hub for long, multi-session agentic projects. It maps an entire project
as a graph of typed tasks (code, question, refactor, review, research, …) and acts as the persistent shared
memory between agent and human sessions: what was done, what's next, and what we know.

Shepherd never spawns or orchestrates agents — it is a passive central hub (storage + REST API +
visualization) that agentic tools query for their next task and report back to.

## Design docs

| Doc | Contents |
|---|---|
| [00 — Scope](docs/00-scope.md) | Problem, boundaries, bbq relationship, v1 scope, non-goals, acceptance criteria |
| [01 — Architecture](docs/01-architecture.md) | Rust core + TS UI, monorepo layout, spec-first pipeline, runtime model |
| [02 — Domain model](docs/02-domain-model.md) | Tasks, relations, lifecycle, claims, sessions, knowledge, invariants |
| [03 — API contract](docs/03-api.md) | REST design rules, resource map, agent loop, SSE catalog, error model |
| [04 — UI](docs/04-ui.md) | Screens, the two graph lenses, liveness, post-project review |
| [05 — Testing](docs/05-testing.md) | Eight-layer taxonomy, staged CI |
| [06 — Roadmap](docs/06-roadmap.md) | v1 in nine CI-green sessions + follow-ups ledger |
