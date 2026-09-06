# Shepherd — Project Scope

> Reference document for [issue #2 — Project scope definition](https://github.com/apkg-ai/shepherd/issues/2).
> Every decision below was agreed during the initial scoping session (2026-09-06). This document is the
> source of truth for the first implementation tasks.

## 1. Purpose & problem statement

**Problem.** Agentic coding today is locked into short, isolated tasks (~2–3 hours). Each session starts
cold: there is no follow-up between tasks, no shared information between agent sessions, and no way to see
where a piece of work sits inside a larger effort.

**What shepherd is.** Shepherd maps an entire project as a graph of typed tasks and acts as the persistent
shared memory and coordination hub across many agent (and human) sessions. It answers three questions at
any point in a project's life: *what was done, what's next, and what do we know* — and it makes the whole
journey reviewable after the project ends.

- **Primary user:** developers running agentic coding tools. First real user: the maintainer, dogfooding on
  the apkg ecosystem. Open source from day one. Not a hosted product.
- **"Long tasks"** means a whole project decomposed into subtasks worked over many sessions — not
  long-running processes or CI pipelines.
- **"Visualisation"** means a local-first browser UI rendering an interactive graph of the project. A
  native app (Tauri) is a possible later step; the architecture keeps that path open.

**Hard boundary: shepherd never spawns or orchestrates agents.** It is a passive central hub — storage,
API, and visualization. Agentic tools query shepherd to know what to do and store their results back.

## 2. Relationship to bbq

**bbq is a skill** (in the Claude-skill sense, à la Matt Pocock's `grill-me`) that embodies the
project-decomposition methodology: interviewing, splitting a large goal into a task graph, and feeding it
into shepherd.

- bbq is **not** a code dependency. It lives **outside this repository**.
- The only coupling is the **API contract** defined in this repository (see §3): bbq conforms to it, as
  will any other client.
- Shepherd is fully usable without bbq.

## 3. Stack & technology choices

| Area | Choice |
|---|---|
| Core / API server | Rust — `axum` HTTP server |
| Persistence | Embedded SQLite via `sqlx` (single local file) |
| Core architecture | Domain logic in a lib crate (Tauri-embeddable later) + server binary |
| UI | Vite + React + TypeScript |
| Graph rendering | Library chosen at implementation time under the minimal-dependency rule |
| Package manager | npm |
| TS linting | oxlint stack |
| API spec linting | Spectral (or equivalent) |

**Principles:**

- **Spec-first.** The OpenAPI spec (`openapi/`, contained in this repo) is designed *before*
  implementation. It is the contract for bbq, the UI, the future CLI/MCP clients, and the server itself.
- **Minimal dependencies**, always at their latest versions.
- TypeScript + Rust covers every planned surface: browser UI now, Tauri native app later (same UI, same
  core crate — nothing rewritten).

## 4. Scope & features

### v1 (MVP — the full core loop)

- Register multiple projects/repositories in one shepherd instance (central hub).
- Create tasks as a human or as an agent proposal; **human approval gate** on agent proposals.
- Graph visualization with two lenses — decomposition tree and dependency flow — with live status updates
  (SSE).
- Agent-facing REST loop: claim a task (lease-based), receive a **context bundle**, report back sessions,
  knowledge, and artifacts.
- Review queue for agent-completed work.
- Versioned **JSON export/import** of a project (backup, portability, and any external processing).

### Version roadmap

1. **v1** — REST API + browser UI (everything above).
2. **Next** — thin CLI (pure REST client, no business logic).
3. **Then** — MCP server layer over the same REST semantics.
4. **Later** — native app (Tauri) wrapping the same UI and core crate.

### Out of scope for v1

- CLI, MCP server, native app (see roadmap).
- Multi-user support and authentication.
- Custom task-type schemas with validation.
- Notifications.
- The bbq skill itself (external deliverable).
- **Agent spawning/orchestration — never, in any version.**

### The single most important user-facing flow

> Agent asks "what's next" → claims a task → gets its context bundle → works → reports session +
> knowledge + artifacts → human reviews → the graph advances and unblocks the next tasks.

## 5. Inputs & outputs (domain model)

### Task

Common core shared by every task: `id`, `title`, `description`, `status`, links, sessions, artifacts —
plus a `type` string and a structured `metadata` JSON field.

- Built-in types are **conventions over that shape**, not separate schemas: `code`, `question`,
  `refactor`, `review`, `research`. New types cost nothing.
- A **`question` task** is one mechanism regardless of direction: completion = a recorded answer. The
  assignee may be a human (agent asks user) or an agent (user asks agent). Blocking uses normal dependency
  semantics; the answer becomes reusable knowledge.

### Relations (two kinds, deliberately distinct)

- **Decomposition** — parent/child: a big task splits into subtasks.
- **Dependency** — `depends_on` ordering edges ("steps"): A must finish before B starts. Acyclicity is
  enforced on dependency edges; each project reads as a graph from a start to an end.

### Lifecycle

`proposed → approved → ready → in_progress → in_review → done`, with `blocked` / `cancelled` reachable
from any state.

- **Claiming** is lease-based: a claim records the agent session and expires on a timeout, so a crashed
  agent never holds a task forever.
- The `in_review` gate is **on by default** for agent-completed tasks, with a per-project toggle.
- **Failure** is a *session outcome*, not a task state: an unsuccessful session records `failed` + reason,
  and the task returns to `ready`. The task keeps its attempt history (failure count + past failed
  sessions), surfaced in the UI so repeated failures are visible at a glance.

### Session records

Structured, not just blobs: caller identity, start/end timestamps, outcome (`succeeded` / `failed` +
reason), summary, decisions made — plus typed attachments (below). A full transcript can be attached for
deep post-project review.

**Caller identity (not auth):** callers self-declare who they are on claim/report — the **harness** (e.g.
Claude Code, Cursor), the **agent/model** (e.g. Opus 5), a session id, and an optional label. Stored on
sessions and claims; humans get an identity too. This is the natural hook for a token layer later.

### Knowledge items (first-class entities)

Typed information items: `link` (GitHub issue/PR/doc), `transcript`, `decision`, `note`/`knowledge`
(refined, reusable content). They attach to a task or session but are **addressable project-wide**, so any
future session or human can pull "everything we know about X" regardless of which task produced it. This
is the mechanism that solves the shared-information problem.

**Project-scoped knowledge:** each project also carries a global set of knowledge items attached to the
project itself (conventions, goals, glossary). These are explicitly **flagged as project-level**, distinct
from task-produced knowledge, and feed the "project-level notes" part of every context bundle. No new
entity type — same knowledge items, different scope.

### Context bundle

What an agent receives when it claims a task:

- the task itself (description, type, metadata);
- summaries and decisions from its dependency ancestors and parent chain;
- project-level notes (conventions, goals);
- linked artifacts;
- in-flight sibling awareness (related tasks currently claimed), to avoid duplicated work.

## 6. Limitations & constraints

- Local single-user daemon; binds to localhost; **no auth in v1** (a token layer can be added if the hub
  ever becomes reachable from other machines).
- Real-time updates over **SSE**.
- Design envelope: dozens of projects, hundreds to low-thousands of tasks per project, a handful of
  concurrent agent sessions.
- **Data location:** a single SQLite file under the user's home (e.g. `~/.shepherd/shepherd.db`,
  XDG-compliant). The JSON export (see §4) is the backup and portability story.
- **Distribution (v1):** clone the repo and build from source (`cargo` + `npm`). Prebuilt release
  binaries come later, alongside the CLI version.

## 7. Expectations & success criteria

**v1 acceptance scenario:**

1. Map one real project into **15+ tasks** across **3+ types**, using both relation kinds.
2. Run **2+ separate Claude Code sessions** against the hub via REST.
3. **Session 2 provably reuses knowledge stored by session 1** — this is *the* acceptance criterion,
   since it is the exact problem shepherd exists to solve.
4. The full start→end graph is visible live in the browser and reviewable after completion.

"Good enough to ship" = the acceptance scenario passes end to end on a real project.

## 8. Project structure & conventions

- **Monorepo layout:** `core/` (Rust cargo workspace: lib crate + server bin), `ui/` (Vite + React + TS),
  `openapi/` (the contract).
- **Branching:** trunk-based; PRs to `main`; squash merge.

### Testing strategy

Strong testing is a stated expectation, split into explicit layers:

| Layer | Scope | Tooling |
|---|---|---|
| **Unit** | Domain lib: lifecycle state machine, DAG operations, lease logic, context-bundle assembly. UI: components and utilities. | `cargo test`; `vitest` + Testing Library |
| **Property-based** | Graph invariants under arbitrary operations: acyclicity of `depends_on`, legal-only lifecycle transitions, claim/lease invariants. | `proptest` |
| **Integration** | axum handlers against a real temp SQLite — full request → handler → DB → response, including SSE event emission and concurrency (N parallel claims on one task → exactly one winner). | `cargo test` (spawned test server) |
| **Contract** | Running server validated against `openapi/shepherd.yaml`: every response conforms to the spec. Spec drift fails CI. The spec is the coupling point for bbq/CLI/MCP, so this layer is critical. | Spectral (static) + dynamic conformance in integration tests |
| **Migration** | Every `sqlx` migration applies cleanly on seeded fixture DBs; existing data survives. A long-lived local hub makes data loss the worst failure mode. | `sqlx` test harness |
| **E2E** | Browser driving the real UI against a real server: register project → create/approve tasks → both graph lenses render → live SSE update → review queue. | Playwright |
| **Smoke** | Seconds-fast boot sanity: server starts on a fresh DB, healthcheck OK, spec served, UI index loads. Gates every PR and release artifact. | Minimal script in CI |
| **Fuzz** | Untrusted input boundaries: task `metadata` JSON, graph mutation payloads, query params. | `cargo-fuzz` — short run per PR, deep run nightly |

- **CI (GitHub Actions):** per PR — `cargo fmt` / `clippy`, unit + property + integration + migration +
  contract, `oxlint` / `tsc` / `vitest`, Spectral, smoke, E2E, short fuzz (~2–3 min per target). Nightly —
  extended fuzz. Coverage reported (`cargo-llvm-cov`, vitest coverage); no hard gate initially.
- **Deferred (post-v1):** load/envelope test (seed low-thousands of tasks, endpoints stay responsive),
  visual regression, accessibility checks (axe in Playwright).

## 9. Open questions & risks

Recorded, non-blocking:

- Graph rendering library (React Flow vs lighter alternatives) — decided at UI implementation under the
  minimal-deps rule.
- Formatting tooling alongside oxlint (oxc formatter maturity) — decided at scaffolding time.
- REST resource naming and shapes — deferred to the spec-first design task, the first implementation task
  after this issue closes.
- bbq skill design — external, tracked outside this repo; only constraint is conformance to the API
  contract.
- Tauri packaging validation — deferred until the native-app version.
