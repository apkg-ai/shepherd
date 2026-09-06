# 01 — Architecture

> How shepherd is built. Scope and boundaries in [00-scope.md](00-scope.md); entity semantics in
> [02-domain-model.md](02-domain-model.md); wire contract in [03-api.md](03-api.md).

## Stack

| Area | Choice | Rationale |
|---|---|---|
| Core / API server | Rust — `axum` (+ `tokio`) | Long-lived local daemon: cheap, stable, Tauri-embeddable |
| Persistence | Embedded SQLite via `sqlx` | Single local file, zero ops, compile-time-checked queries |
| UI | Vite + React + TypeScript | Standard, fast iteration; reused as-is by Tauri later |
| Graph rendering | Decided in S8 under the minimal-dependency rule | React Flow vs lighter alternatives |
| Package manager | npm | House choice |
| TS linting | oxlint stack | House choice |
| Spec linting | Spectral (or equivalent) | Contract-first gate |

**Principles:** minimal dependencies, always at latest versions. TypeScript + Rust covers every planned
surface — browser UI now, Tauri native app later, same UI + same core crate, nothing rewritten.

## Monorepo layout

```
shepherd/
├── core/                  # cargo workspace
│   ├── shepherd-core/     # lib crate: domain, storage, services (Tauri-embeddable)
│   └── shepherd-server/   # bin crate: axum routing, SSE, static UI serving — thin
├── ui/                    # Vite + React + TS
├── openapi/               # shepherd.yaml — THE contract (spec-first)
└── docs/                  # this suite
```

- **`shepherd-core`** owns everything that matters: entities, lifecycle state machine, DAG operations,
  lease logic, context-bundle assembly, SQLite access, export/import. No HTTP types leak into it.
- **`shepherd-server`** is deliberately thin: route definitions, request/response mapping to the OpenAPI
  shapes, SSE fan-out, serving the built UI. If logic appears here, it belongs in the lib.
- This split is the **Tauri insurance**: the native app embeds `shepherd-core` directly and reuses `ui/`.

## Spec-first pipeline

1. `openapi/shepherd.yaml` is written **before** handlers ([03-api.md](03-api.md) sets the design rules;
   session S2 produces the spec).
2. Spectral lints the spec in CI (static gate).
3. Integration tests validate live responses against the spec (dynamic gate — spec drift fails CI, see
   [05-testing.md](05-testing.md)).
4. The UI's API client is derived from / checked against the spec. External clients (bbq, future CLI/MCP)
   build against the same file.

## Runtime model

- One daemon: `shepherd-server` binds `127.0.0.1:<port>` (default port TBD in S1; overridable by flag/env).
- Serves: REST under `/api/v1`, SSE under `/api/v1/events`, the built UI at `/`, `GET /health`, and the
  spec itself (`/api/v1/openapi.yaml`).
- **Data location:** `~/.shepherd/shepherd.db` (XDG-compliant; `SHEPHERD_DB` override for tests/dev).
- **Config surface (v1, deliberately tiny):** port, DB path, review-gate default. Flags/env only — no
  config file until something needs it.
- **SSE mechanics:** a `tokio::sync::broadcast` channel in `shepherd-core` emits typed domain events on
  every mutation; the server fans out to SSE subscribers. No replay/`Last-Event-ID` in v1 — the UI
  refetches on reconnect. Event catalog in [03-api.md](03-api.md).
- **Export/import:** versioned JSON document per project (schema version embedded); export is a pure read,
  import creates a new project (no merge semantics in v1).

## Dev workflow

- `cargo run -p shepherd-server` + `npm run dev` in `ui/` (Vite proxies `/api` to the server).
- Production build: `npm run build` → static assets served by the server binary.
- Tests, lint, and CI stages: [05-testing.md](05-testing.md).

## Conventions

- Trunk-based; PRs to `main`; squash merge.
- Rust: `rustfmt` + `clippy` (deny warnings in CI); error handling with `thiserror` (lib) — final crate
  choices recorded in S1.
- TS: oxlint; formatter chosen in S1 (oxc formatter if mature enough, else Prettier).
- Every roadmap session ([06-roadmap.md](06-roadmap.md)) ends CI-green and shippable.
