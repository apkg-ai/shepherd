# Agent Integration Guide

> How any agent harness drives shepherd via REST. The authoritative contract is
> [`openapi/shepherd.yaml`](../openapi/shepherd.yaml); this guide walks the
> patterns. See also the SSE event catalog in
> [`openapi/shepherd-events.asyncapi.yaml`](../openapi/shepherd-events.asyncapi.yaml).

## Prerequisites

Start the shepherd daemon:

```bash
cargo run -p shepherd-server
# Default: http://127.0.0.1:7437
```

The server binds to localhost on port 7437 (override with `--port` or
`SHEPHERD_PORT`). v1 has no authentication — identity is self-declared in
request bodies, not enforced.

## The agent loop

The core workflow every agent follows. This matches the contract in
[03-api.md](03-api.md#the-agent-loop-in-contract-terms).

```
┌─ 1. GET  next-task ──────── what's next?
│  2. POST claim ──────────── lease it
│  3. GET  context ────────── get the bundle
│  4.      (work) ─────────── outside shepherd
│  5. POST sessions ───────── report back
│  ↻  POST claim/renew ────── extend lease while working
└─    POST claim/release ──── give up without reporting
```

### 1. Find work

Ask shepherd for the highest-priority `ready`, unclaimed task:

```bash
curl -s http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/next-task | jq .
```

```typescript
const res = await fetch(
  `${BASE}/api/v1/projects/${projectId}/next-task`
);
const { task } = await res.json();

if (!task) {
  console.log("Nothing to do");
  return;
}
```

Returns `{ "task": { ... } }` or `{ "task": null }` when nothing is available.

### 2. Claim the task

Acquire a lease with your identity and a TTL:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/claim \
  -H "Content-Type: application/json" \
  -d '{
    "identity": {
      "harness": "claude-code",
      "agent_model": "opus-5",
      "session_id": "sess-20260906-abc123",
      "label": "S3 domain core"
    },
    "ttl_seconds": 600
  }' | jq .
```

```typescript
const identity = {
  harness: "claude-code",
  agent_model: "opus-5",
  session_id: `sess-${Date.now()}`,
  label: "S3 domain core",
};

const claim = await fetch(
  `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/claim`,
  {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ identity, ttl_seconds: 600 }),
  }
).then((r) => r.json());
```

The task transitions to `in_progress`. If someone else already holds the claim,
you get a `409 Conflict` with `urn:shepherd:error:claim-conflict`.

**Invariant:** at most one active claim per task. Expired leases release the
task back to `ready` automatically — no task is held forever.

### 3. Get context

Fetch the assembled context bundle:

```bash
curl -s \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/context \
  | jq .
```

```typescript
const ctx = await fetch(
  `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/context`
).then((r) => r.json());

// ctx.task             — the task itself
// ctx.ancestor_summaries — what predecessor tasks produced
// ctx.project_knowledge  — conventions, goals, glossary
// ctx.artifacts          — linked commits, PRs, docs
// ctx.sibling_tasks      — what other sessions are working on right now
```

The context bundle solves the shared-information problem: session 2 sees
everything session 1 produced.

### 4. Work

Work happens outside shepherd — writing code, running tests, thinking. The
agent uses the context bundle to understand what to do and why.

### 5. Report results

When done, record a session:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "identity": {
      "harness": "claude-code",
      "agent_model": "opus-5",
      "session_id": "sess-20260906-abc123",
      "label": "S3 domain core"
    },
    "started_at": "2026-09-06T10:10:00Z",
    "ended_at": "2026-09-06T12:15:00Z",
    "outcome": "succeeded",
    "summary": "Implemented lifecycle state machine with all transitions.",
    "decisions": [
      "Use a state-transition table rather than match arms."
    ],
    "knowledge_items": [
      {
        "type": "decision",
        "title": "State-transition table design",
        "content": "Transitions encoded as static lookup table indexed by (current_status, trigger)."
      }
    ],
    "artifacts": [
      "https://github.com/apkg-ai/shepherd/pull/25"
    ]
  }' | jq .
```

```typescript
const session = await fetch(
  `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/sessions`,
  {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      identity,
      started_at: startTime,
      ended_at: new Date().toISOString(),
      outcome: "succeeded",
      summary: "Implemented lifecycle state machine.",
      decisions: ["Use a state-transition table."],
      knowledge_items: [
        {
          type: "decision",
          title: "State-transition table design",
          content: "Transitions as a static lookup table.",
        },
      ],
      artifacts: ["https://github.com/apkg-ai/shepherd/pull/25"],
    }),
  }
).then((r) => r.json());
```

**What happens on report:**

- `outcome: "succeeded"` + review gate **on** → task → `in_review`
- `outcome: "succeeded"` + review gate **off** → task → `done`
- `outcome: "failed"` → task → `ready` (claimable again, attempt count
  increments, failure history kept)

### 6. Renew lease (while working)

If work takes longer than the TTL, extend the lease:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/claim/renew \
  -H "Content-Type: application/json" \
  -d '{
    "identity": {
      "harness": "claude-code",
      "agent_model": "opus-5",
      "session_id": "sess-20260906-abc123"
    },
    "ttl_seconds": 600
  }' | jq .
```

```typescript
await fetch(
  `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/claim/renew`,
  {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ identity, ttl_seconds: 600 }),
  }
);
```

### 7. Release claim (giving up)

Voluntarily release without reporting a session:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/claim/release \
  -H "Content-Type: application/json" \
  -d '{
    "identity": {
      "harness": "claude-code",
      "agent_model": "opus-5",
      "session_id": "sess-20260906-abc123"
    }
  }' | jq .
```

The task returns to `ready` and becomes claimable by others.

## Creating work

### Propose a task (agent)

Agents create tasks with `status: "proposed"` — they require human approval:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Add rate-limit headers to responses",
    "description": "Implement RFC 6585 rate-limit headers on all endpoints.",
    "type": "code",
    "status": "proposed",
    "metadata": { "priority": "medium" }
  }' | jq .
```

### Create directly (human)

Humans can create tasks at `approved` — they skip the proposal gate:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Design the review queue UI",
    "type": "code",
    "status": "approved"
  }' | jq .
```

### Add relations

Create decomposition (parent/child) or dependency edges:

```bash
# This task depends on another task (prerequisite ordering)
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/tasks/${TASK_ID}/relations \
  -H "Content-Type: application/json" \
  -d '{
    "type": "depends_on",
    "target_task_id": "019263a0-1234-7def-8000-000000000011"
  }' | jq .
```

**Invariant:** the dependency graph is always acyclic. Any relation that would
create a cycle is rejected with `409 Conflict` and
`urn:shepherd:error:dependency-cycle`.

## Knowledge

### Record knowledge

Attach reusable information at any scope:

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/knowledge \
  -H "Content-Type: application/json" \
  -d '{
    "type": "decision",
    "title": "Cursor pagination over offset",
    "content": "All list endpoints use cursor-based pagination for stable results under concurrent writes.",
    "scope": "project"
  }' | jq .
```

Knowledge item types:

| Type | Use |
|---|---|
| `link` | Issue, PR, commit, doc, file |
| `transcript` | Session transcript for deep review |
| `decision` | A recorded decision and its rationale |
| `note` | Refined, reusable content |

### Query knowledge

```bash
# All project-level knowledge
curl -s "http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/knowledge?scope=project" | jq .

# All knowledge produced by a specific task
curl -s "http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/knowledge?task_id=${TASK_ID}" | jq .
```

## Listening for changes (SSE)

Open an SSE stream for real-time updates:

```bash
curl -N "http://127.0.0.1:7437/api/v1/events?project_id=${PROJECT_ID}"
```

```typescript
const es = new EventSource(
  `${BASE}/api/v1/events?project_id=${projectId}`
);

es.addEventListener("task.status_changed", (e) => {
  const data = JSON.parse(e.data);
  console.log(`Task ${data.task_id}: ${data.old_status} → ${data.new_status}`);
});

es.addEventListener("knowledge.added", (e) => {
  const data = JSON.parse(e.data);
  console.log(`New knowledge: ${data.type} in ${data.scope}`);
});
```

Event types: `project.created`, `project.updated`, `task.created`,
`task.updated`, `task.status_changed`, `relation.added`, `relation.removed`,
`claim.acquired`, `claim.renewed`, `claim.released`, `claim.expired`,
`session.recorded`, `knowledge.added`.

No replay / `Last-Event-ID` in v1 — refetch on reconnect.

## Export / Import

### Export a project

```bash
curl -s http://127.0.0.1:7437/api/v1/projects/${PROJECT_ID}/export > backup.json
```

The document carries its own schema version (`"1.0.0"`) for forward
compatibility.

### Import from a backup

```bash
curl -s -X POST \
  http://127.0.0.1:7437/api/v1/projects/import \
  -H "Content-Type: application/json" \
  -d @backup.json | jq .
```

Import always creates a **new project** — no merge semantics in v1. Claims
are runtime state and are not exported: any `in_progress` task in the
document is normalized on import to `ready` (all its dependencies `done`) or
`approved`, so it is immediately claimable in the new project.

## Error handling

All errors use [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)
`application/problem+json` with stable URN type slugs:

```json
{
  "type": "urn:shepherd:error:dependency-cycle",
  "title": "Dependency Cycle",
  "status": 409,
  "detail": "Adding this relation would create a cycle: A → B → C → A"
}
```

### Error type URNs

| URN | HTTP | When |
|---|---|---|
| `urn:shepherd:error:not-found` | 404 | Resource does not exist |
| `urn:shepherd:error:validation-error` | 422 | Request body failed validation |
| `urn:shepherd:error:dependency-cycle` | 409 | Relation would create a cycle |
| `urn:shepherd:error:claim-conflict` | 409 | Task is already claimed |
| `urn:shepherd:error:invalid-transition` | 409 | Status transition is not allowed |
| `urn:shepherd:error:already-claimed` | 409 | Duplicate claim attempt |
| `urn:shepherd:error:lease-expired` | 410 | Claim TTL elapsed |
| `urn:shepherd:error:task-not-ready` | 409 | Task is not in `ready` status |
| `urn:shepherd:error:import-schema-mismatch` | 422 | Export document version incompatible |
| `urn:shepherd:error:internal-error` | 500 | Unexpected server error |

### TypeScript error handling

```typescript
async function shepherdFetch(url: string, init?: RequestInit) {
  const res = await fetch(url, init);

  if (!res.ok) {
    const problem = await res.json();
    throw new ShepherdError(problem);
  }

  // 204 No Content has no body
  if (res.status === 204) return null;
  return res.json();
}

class ShepherdError extends Error {
  type: string;
  status: number;
  detail?: string;

  constructor(problem: { type: string; title: string; status: number; detail?: string }) {
    super(problem.title);
    this.type = problem.type;
    this.status = problem.status;
    this.detail = problem.detail;
  }
}
```

## Identity

Identity is self-declared, not authenticated. Every claim and session report
carries the caller's identity:

| Field | Required | Description |
|---|---|---|
| `harness` | yes | The tool making the call (e.g. `claude-code`, `cursor`) |
| `agent_model` | yes | The model or person (e.g. `opus-5`, `human`) |
| `session_id` | yes | Caller-generated session correlation ID |
| `label` | no | Human-readable session label |

Identity is stored on claims and sessions. Humans get an identity too. This is
the natural hook for a token-based auth layer in the future (follow-up F4).

## Complete TypeScript example

A self-contained loop that claims the next available task, works on it, and
reports back:

```typescript
const BASE = "http://127.0.0.1:7437";

interface Identity {
  harness: string;
  agent_model: string;
  session_id: string;
  label?: string;
}

async function agentLoop(projectId: string, identity: Identity) {
  // 1. Find work
  const { task } = await fetch(
    `${BASE}/api/v1/projects/${projectId}/next-task`
  ).then((r) => r.json());

  if (!task) {
    console.log("No tasks available");
    return;
  }

  console.log(`Found task: ${task.title} (${task.id})`);

  // 2. Claim it
  const claim = await fetch(
    `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/claim`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ identity, ttl_seconds: 600 }),
    }
  ).then((r) => {
    if (!r.ok) throw new Error(`Claim failed: ${r.status}`);
    return r.json();
  });

  console.log(`Claimed until ${claim.expires_at}`);

  // 3. Get context
  const ctx = await fetch(
    `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/context`
  ).then((r) => r.json());

  console.log(`Context: ${ctx.ancestor_summaries.length} ancestors, ` +
    `${ctx.project_knowledge.length} knowledge items`);

  // 4. Work (simulated)
  const startTime = new Date().toISOString();
  // ... actual work happens here ...
  const endTime = new Date().toISOString();

  // 5. Report
  const session = await fetch(
    `${BASE}/api/v1/projects/${projectId}/tasks/${task.id}/sessions`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        identity,
        started_at: startTime,
        ended_at: endTime,
        outcome: "succeeded",
        summary: "Completed the task successfully.",
        decisions: [],
        knowledge_items: [],
        artifacts: [],
      }),
    }
  ).then((r) => r.json());

  console.log(`Session recorded: ${session.id} (${session.outcome})`);
}

// Run it
agentLoop("019263a0-1234-7def-8000-000000000001", {
  harness: "claude-code",
  agent_model: "opus-5",
  session_id: `sess-${Date.now()}`,
  label: "example agent loop",
});
```

## Graph representation

Shepherd uses a **hybrid** graph representation:

- **Derived** (default): start tasks = no incoming `depends_on` edges, end
  tasks = no outgoing `depends_on` edges. The query layer computes these.
- **Explicit** (optional): tasks may carry a `graph_role` array with values
  `start`, `end`, or `milestone`. A single-task project can have
  `["start", "end"]`. Explicit roles override the derived computation for
  UI/visualization anchoring.

This decision is recorded per the S2 issue requirements.
