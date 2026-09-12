# 03 — API contract (design rules)

> The actual contract is `openapi/shepherd.yaml`, written in session **S2** — before any handler exists.
> This document fixes the rules that spec must follow, so S2 is design work, not invention.

## Principles

1. **The spec is the product boundary.** bbq, the UI, the future CLI and MCP layer are all clients of the
   same contract. Anything not in the spec does not exist.
2. **Boring REST.** Plural nouns, standard verbs, no RPC-isms except where the domain is an action
   (claim, approve — modeled as sub-resources/actions below).
3. **Versioned from day one:** everything under `/api/v1`. The export document carries its own schema
   version independently.
4. **Errors:** RFC 9457 `application/problem+json` everywhere, with stable machine-readable `type` slugs
   (e.g. `dependency-cycle`, `claim-conflict`, `invalid-transition`).
5. **Identity travels in request bodies** for claim/report calls (harness, agent/model, session id,
   label) — not headers — so it is visible in the spec, testable, and later swappable for authenticated
   identity without URL changes.
6. **Pagination** on all list endpoints (cursor-based), even if v1 volumes don't need it — retrofitting
   breaks clients.

## Resource map (shape, not final naming — S2 decides details)

```
/api/v1
├── /projects                        CRUD, settings (review gate), registration
│   └── /{project}/export|import    versioned JSON document
├── /projects/{project}/tasks        CRUD; create as proposal or approved
│   └── /{task}
│       ├── /approve|reject          proposal + review-queue actions
│       ├── /claim                   lease: acquire / renew / release
│       ├── /context                 the context bundle
│       ├── /sessions                report work episodes (outcome, decisions, knowledge)
│       └── /relations               decomposition + depends_on (cycle-rejecting)
├── /projects/{project}/relations    bulk read of every relation — the graph view's feed
├── /projects/{project}/next-task    "what's next" query for agents (ready, unclaimed, priority order)
├── /projects/{project}/knowledge    project-wide knowledge (incl. project-scoped set)
├── /events                          SSE stream (see catalog)
├── /health
└── /openapi.yaml                    the served contract
```

## The agent loop, in contract terms

1. `GET next-task` → task id (or nothing to do)
2. `POST claim` (identity + TTL) → lease
3. `GET context` → context bundle
4. work happens outside shepherd
5. `POST sessions` (outcome, summary, decisions, knowledge items, artifacts) → task advances
   (`in_review`/`done`/back to `ready` on failure per [02](02-domain-model.md))

This loop is what the acceptance test drives via plain REST, and what the **agent integration guide**
(written in S2 alongside the spec) documents for any harness.

## SSE event catalog (v1)

Typed domain events, one stream, project-filterable:
`project.created|updated`, `task.created|updated|status_changed`, `relation.added|removed`,
`claim.acquired|renewed|released|expired`, `session.recorded`, `knowledge.added`.

No replay / `Last-Event-ID` in v1 — clients refetch on reconnect ([01](01-architecture.md)).

## Conformance

- Spectral lints the spec (static, CI gate).
- Integration tests validate every live response against the spec (dynamic, CI gate) — spec drift fails
  the build. See [05-testing.md](05-testing.md).
