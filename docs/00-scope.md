# 00 — Shepherd: Scope

> Reference for [issue #2 — Project scope definition](https://github.com/apkg-ai/shepherd/issues/2).
> Agreed during the scoping session of 2026-09-06. Source of truth for all implementation work.
>
> **Reading order:** [00 scope](00-scope.md) → [01 architecture](01-architecture.md) →
> [02 domain model](02-domain-model.md) → [03 API contract](03-api.md) → [04 UI](04-ui.md) →
> [05 testing](05-testing.md) → [06 roadmap](06-roadmap.md).

## Problem

Agentic coding today is locked into short, isolated tasks (~2–3 hours). Each session starts cold: there is
no follow-up between tasks, no shared information between agent sessions, and no way to see where a piece
of work sits inside a larger effort.

## What shepherd is

Shepherd maps an entire project as a graph of typed tasks and acts as the persistent shared memory and
coordination hub across many agent (and human) sessions. It answers three questions at any point in a
project's life: **what was done, what's next, and what do we know** — and it makes the whole journey
reviewable after the project ends.

- **Primary user:** developers running agentic coding tools. First real user: the maintainer, dogfooding
  on the apkg ecosystem. Open source from day one. Not a hosted product.
- **"Long tasks"** = a whole project decomposed into subtasks worked over many sessions — not long-running
  processes or CI pipelines.
- **"Visualisation"** = a local-first browser UI rendering an interactive graph of the project. A native
  app (Tauri) is a later step; the architecture keeps that path open ([01](01-architecture.md)).

## Hard boundaries

1. **Shepherd never spawns or orchestrates agents — in any version.** It is a passive central hub:
   storage, API, visualization. Agentic tools query shepherd to know what to do and store results back.
2. **Local-first, single user.** Binds to localhost, no auth in v1 (identity ≠ auth — see
   [02](02-domain-model.md)). A token layer is a follow-up if the hub ever becomes remotely reachable.
3. **Spec-first.** The OpenAPI contract is designed before implementation and is the coupling point for
   every client ([03](03-api.md)).

## Relationship to bbq

**bbq is a skill** (in the Claude-skill sense, à la Matt Pocock's `grill-me`) embodying the
project-decomposition methodology: interviewing, splitting a large goal into a task graph, feeding it into
shepherd.

- bbq is **not** a code dependency. It lives **outside this repository**.
- The only coupling is the API contract defined here: bbq conforms to it, as will any other client.
- Shepherd is fully usable without bbq.

## v1 scope (MVP — the full core loop)

- Register multiple projects/repositories in one shepherd instance (central hub).
- Create tasks as a human or as an agent proposal; **human approval gate** on agent proposals.
- Graph visualization with two lenses — decomposition tree and dependency flow — live via SSE.
- Agent-facing REST loop: claim a task (lease-based), receive a **context bundle**, report back sessions,
  knowledge, and artifacts.
- Review queue for agent-completed work.
- Versioned **JSON export/import** of a project (backup, portability, external processing).

### The single most important user-facing flow

> Agent asks "what's next" → claims a task → gets its context bundle → works → reports session +
> knowledge + artifacts → human reviews → the graph advances and unblocks the next tasks.

### Version roadmap

1. **v1** — REST API + browser UI (everything above). Sessions S1–S9 in [06-roadmap.md](06-roadmap.md).
2. **Next** — thin CLI (pure REST client, no business logic).
3. **Then** — MCP server layer over the same REST semantics.
4. **Later** — native app (Tauri) wrapping the same UI and core crate.

### Out of scope for v1

- CLI, MCP server, native app (see roadmap issues).
- Multi-user support and authentication.
- Custom task-type schemas with validation.
- Notifications.
- The bbq skill itself (external deliverable).
- **Agent spawning/orchestration — never, in any version.**

## Constraints & envelope

- Local single-user daemon on localhost; SSE for real-time updates.
- Design envelope: dozens of projects, hundreds to low-thousands of tasks per project, a handful of
  concurrent agent sessions.
- Data: a single SQLite file under the user's home (`~/.shepherd/shepherd.db`, XDG-compliant); JSON
  export is the backup and portability story.
- Distribution (v1): clone and build from source (`cargo` + `npm`). Prebuilt binaries are a follow-up.

## Acceptance criteria (v1 is done when…)

1. One real project is mapped into **15+ tasks** across **3+ types**, using both relation kinds.
2. **2+ separate Claude Code sessions** run against the hub via REST.
3. **Session 2 provably reuses knowledge stored by session 1** — *the* acceptance criterion, since it is
   the exact problem shepherd exists to solve.
4. The full start→end graph is visible live in the browser and reviewable after completion.

"Good enough to ship" = this scenario passes end to end on a real project.
