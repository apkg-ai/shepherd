# 02 — Domain model

> Entity semantics and invariants. The wire representation lives in the OpenAPI spec
> ([03-api.md](03-api.md), produced in S2); this document is the meaning behind it.

## Entities

| Entity | Role |
|---|---|
| **Project** | Registered repository/effort; owns tasks, project-scoped knowledge, settings (e.g. review-gate toggle) |
| **Task** | Unit of work, typed, positioned in the graph |
| **Relation** | Decomposition (parent/child) or dependency (`depends_on`) between tasks |
| **Claim** | Lease binding a task to a caller identity for a bounded time |
| **Session** | One work episode on a task: identity, timestamps, outcome, decisions, attachments |
| **Knowledge item** | Typed, reusable information attached to a task, session, or the project itself |
| **Identity** | Self-declared caller descriptor (not auth): harness, agent/model, session id, label |

## Task

Common core shared by every task: `id`, `title`, `description`, `status`, optional `assignee` (an
identity — human or agent), relations, sessions, knowledge — plus a **`type` string** and a structured
**`metadata` JSON** field.

- Built-in types are **conventions over that shape**, not separate schemas: `code`, `question`,
  `refactor`, `review`, `research`. New types cost nothing; formal per-type schemas with validation are a
  post-v1 follow-up.
- **`question` task** — one mechanism regardless of direction: completion = a recorded answer. Assignee
  may be a human (agent asks user) or an agent (user asks agent). Blocking uses normal dependency
  semantics; the answer becomes reusable knowledge.
- **"Artifacts"** are not a separate entity: they are knowledge items of type `link` (commits, PRs,
  files) attached to the task.

## Relations — two kinds, deliberately distinct

- **Decomposition** — parent/child: a big task splits into subtasks. A task has at most one parent.
- **Dependency** — `depends_on` edges ("steps"): A must finish before B starts.

**Invariant:** the dependency graph is acyclic — every mutation that would create a cycle is rejected.
Each project reads as a graph from a start to an end (explicit vs derived representation decided in S2).

## Lifecycle

```
proposed → approved → ready → in_progress → in_review → done
```

| Transition | Trigger |
|---|---|
| `proposed → approved` | Human approves an agent proposal (human-created tasks may start at `approved`) |
| `approved → ready` | **Automatic**: every `depends_on` task is `done` |
| `ready → in_progress` | A caller claims the task |
| `in_progress → in_review` | Claimant reports a successful session (review gate on — default) |
| `in_progress → done` | Same, when the project's review gate is off |
| `in_review → done` | Human approves the work |
| `in_review → ready` | Human rejects — task is claimable again, history kept |
| any → `blocked` / back | **Explicit flag** with a reason, raised by human or agent. Waiting on dependencies is *not* `blocked` |
| any → `cancelled` | Human decision; terminal |

- **Failure is a session outcome, not a task state**: an unsuccessful session records `failed` + reason
  and the task returns to `ready`. The task keeps its attempt history (failure count + past failed
  sessions), surfaced in the UI so repeated failures are visible at a glance.

## Claims (leases)

- Claiming binds a task to an identity with a **TTL lease**; the claimant may renew while working.
- Lease expiry (crashed agent) releases the task back to `ready` automatically — no task is held forever.
- **Invariant:** at most one active claim per task; N concurrent claim attempts → exactly one winner.

## Sessions

Structured records, not blobs: caller identity, start/end timestamps, outcome (`succeeded` / `failed` +
reason), summary, decisions made — plus attached knowledge items. A full transcript can be attached for
deep post-project review.

**Caller identity (not auth):** callers self-declare on claim/report — the **harness** (e.g. Claude Code,
Cursor), the **agent/model** (e.g. Opus 5), a session id, an optional label. Stored on sessions and
claims; humans get an identity too. This is the natural hook for a token layer later.

## Knowledge items

Typed: `link` (issue/PR/commit/doc), `transcript`, `decision`, `note`/`knowledge` (refined, reusable
content).

- Attached to a task or session but **addressable project-wide**: any future session or human can pull
  "everything we know about X" regardless of which task produced it. This is the mechanism that solves the
  shared-information problem.
- **Project-scoped knowledge:** each project also carries a global knowledge set attached to the project
  itself (conventions, goals, glossary) — explicitly flagged as project-level, distinct from task-produced
  knowledge. Same entity, different scope.

## Context bundle

What a caller receives on claim (assembled by `shepherd-core`):

1. the task itself (description, type, metadata);
2. summaries and decisions from its dependency ancestors and parent chain;
3. project-level knowledge (conventions, goals);
4. linked artifacts;
5. in-flight sibling awareness (related tasks currently claimed) — prevents duplicated work.

## Invariants (feed the property-based tests — [05-testing.md](05-testing.md))

1. Dependency graph is always acyclic.
2. Task status changes only along the transition table above.
3. At most one active claim per task; expired leases always release.
4. `ready` ⇔ `approved` + all dependencies `done` (and not blocked/cancelled).
5. A task's attempt history is append-only.
6. Export → import round-trips a project losslessly (same schema version).
