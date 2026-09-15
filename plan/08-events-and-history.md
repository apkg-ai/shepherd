# Durable events and history (DATA-01)

## Storage and delivery

events.id is SQLite INTEGER PRIMARY KEY AUTOINCREMENT; wire cursor is decimal string to avoid JS number precision loss. Row carries project_id, actor_id, command_id, type, resource_id, resource_revision, occurred_at, affected_ids JSON, action (operationId or system.expire/system.import), and reason (command reason or empty string). Unblock events preserve the cleared block reason; cancellation, archive, waiver and withdrawal events preserve the provided reason. These durable explanations survive idempotency expiry. System expiry uses a fixed system actor record (kind=human for historical wire compatibility, label=System, revoked=true, no credentials); it cannot authenticate. Credential actions are stored in separate security_audit table because they are not project-scoped.

Use type catalog in [AsyncAPI](contracts/asyncapi.yaml). A `.changed` event represents create/update/status/phase/archive; clients fetch current resource, not apply patches. claim.changed includes task ID in affected_ids; dependency.changed includes both endpoints and affected scope; epic/task changes include owning goal/epic IDs. Stable command_id groups cascades; order within a command is deterministic by resource kind then UUID, with command-local changes coalesced once per resource. session.recorded and review.recorded remain separate immutable records.

SSE route /api/v1/projects/{project_id}/events is authenticated read. Header Last-Event-ID takes precedence over after query. No cursor: establish current high water and stream subsequent events; clients load initial query snapshots after opening the subscription to avoid a gap. Frame: id: <decimal>, event: change, data: <Event JSON>. Every 15 seconds send a comment heartbeat, no ID. Native EventSource cookie auth is used by browser; CLI/MCP do not need SSE for core operations.

Replay uses persisted rows with project filter and id>cursor, max 1,000 per connection establishment. Subscribe wakeup before reading high-water/replay, then query again after each notification; this prevents commit-between-replay-and-subscribe gaps. Global numeric gaps from other projects are valid. If cursor > global max, cursor predates retained minimum, or backlog >1,000, emit event: resync_required with current global cursor/reason; close stream. Client refetches all project data and reconnects with that cursor. Disconnect lagging clients after 256 queued notifications; retained history still recovers them.

Retain audit events indefinitely in v1; replay accepts only last 7 days, allowing bounded query cost without deleting audit history. History endpoint reads all retained rows, newest first. Busy/timeout during streaming closes connection; no fabricated success. Restart resumes IDs from DB. Backup restore can roll cursor backwards, triggering cursor_ahead and full resync.

## Event consumers

| Event | Invalidate |
|---|---|
| project.changed | project registry, project detail/settings |
| goal.changed | goal list/detail, project counts |
| epic.changed | epic detail/list, goal graph/counts, project counts, work list |
| task.changed | task detail/list/context, epic graph/counts, work and proposal/review queues |
| dependency.changed | scoped graph, dependency lists, affected contexts/work/eligibility |
| claim.changed | claims, task/context, work and review queues; status style without relayout |
| session.recorded | sessions, task/context/history |
| document.changed | documents/revisions/context; preserve unsaved editors |
| submission.changed | submission/detail/queue, task/context |
| review.recorded | review history, submission/detail/queue, task/context |
| task_type.changed | type registry and task forms/lists |

Every project event also invalidates history. Payload references are addressable even after archive; no secrets or document bodies in SSE. Permission changes require reauthentication checks during subsequent requests; revoked bearer stream closes on periodic heartbeat recheck.
