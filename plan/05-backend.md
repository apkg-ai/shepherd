# Backend implementation specification

## Stack and module boundaries

Keep Rust 2024, Tokio, Axum, SQLx/SQLite and current crate versions/lockfiles. Core is independent of transport; generated wire models remain server-side and convert explicitly. Add a shared HTTP client crate later, not a dependency from core to server.

```text
core/shepherd-core/src/
  model/{mod,project,goal,epic,task,claim,content,review,identity,event}.rs
  workflow/{mod,epic,task,eligibility,policy}.rs
  storage/{mod,connect,transaction,rows}.rs
  commands/{mod,hierarchy,dependencies,claims,reports,content,reviews,identity,archive}.rs
  queries/{mod,hierarchy,work,context,history,graph}.rs
  dag.rs, lease.rs, export.rs, error.rs, lib.rs
core/shepherd-server/src/
  handlers/{mod,hierarchy,claims,content,reviews,identity,events,portability}.rs
  convert/{mod,hierarchy,work,content,errors}.rs
  generated/, middleware.rs, main.rs, lib.rs
```

Do not create all files empty in the first step. Introduce each with its behavior and tests. Store becomes a facade holding pool/clock/notifier/key provider, not the place for every method implementation. Command helpers accept the existing transaction; never open nested independent writes. Query helpers take an Executor or read transaction so export/context can use one snapshot.

## Interfaces (DATA-01)

```rust
struct CommandContext { actor: Actor, command_id: CommandId,
    idempotency_key: Uuid, expected_revision: Option<i64>, now: DateTime<Utc> }
struct CommandResult<T> { value: T, events: Vec<EventId> }
async fn claim_task(&self, ctx: CommandContext, project: ProjectId,
    task: TaskId, input: ClaimInput) -> Result<CommandResult<ClaimGrant>>;
async fn report_claim(&self, ctx: CommandContext, project: ProjectId,
    claim: ClaimId, lease: SecretString, input: ReportInput)
    -> Result<CommandResult<Session>>;
fn evaluate_task(snapshot: &TaskSnapshot, actor: &Actor, now: DateTime<Utc>)
    -> Eligibility;
async fn recompute(tx: &mut Transaction<'_, Sqlite>, affected: AffectedScope,
    events: &mut Vec<PendingEvent>, now: DateTime<Utc>) -> Result<()>;
```

Use equivalent method naming for every operation in the REST catalog. SecretString is a local redacting wrapper, not a requirement for another crate. Never Debug-print credentials. Core receives authenticated Actor but rechecks actor revocation and operation capability inside the command transaction. Errors are domain variants, mapped to stable Problem codes in server.

## Command transaction algorithm

1. Acquire SQLx pooled connection (max 8, acquire timeout 5 seconds), begin transaction and immediately obtain SQLite write reservation. Immediately execute UPDATE command_lock SET value=value WHERE id=1 after BEGIN; this selected no-op UPDATE obtains the write reservation. do not defer the write lock until after reads.
2. Read registered actor, check revocation, then idempotency row. Match canonical request hash, decrypt successful response if applicable, and return it on replay.
3. Load all authoritative command resources under that lock. Validate ownership, capability, revisions, lease and state in workflow precedence order.
4. Reconcile expired claims in affected scope. Apply command, append immutable reports/content/reviews, and update mutable resource counters once per resource.
5. Recompute affected epic and successor states in dependency topological order. No async/background cascade is allowed to determine correctness.
6. Append events and successful idempotency response in transaction. Commit once.
7. Notify the in-memory broadcast channel with last event ID. Notification failure cannot roll back committed work; SSE queries persisted history.

Configure SQLite WAL, foreign_keys=ON, synchronous=FULL, busy_timeout=5000. Serialize per database through SQLite, not an in-memory mutex that would fail under a second process. Reject second daemon ownership using a lock file with OS advisory lock; tests still exercise separate pools for race coverage.

## Queries and graph

No write side effects in ordinary GET. Derive expired claim eligibility using query clock. Context and export use snapshot transactions. Graph endpoint returns complete scoped nodes/edges up to 2,000 nodes/4,000 edges; 422 graph_too_large above limit with link to lists. Never return a partially connected graph without indicating it. Paginated collections allow larger project history. Goal graph has epic nodes; epic graph has task nodes. Counts are batched GROUP BY queries; avoid per-node database queries.

Context response budget: 256 KiB UTF-8 JSON; selected plan always first (one max-64K-character revision can approach budget), then task/epic/goal/project, recent 20 sessions/reviews/submissions and project/epic documents newest first. If mandatory fields + selected plan exceed budget return 422 context_too_large with getDocumentRevision continuation; return selected plan metadata via document endpoint rather than silently dropping it. Optional sections truncate deterministically, set truncated=true, and emit exact collection continuation paths. Reference only committed immutable revisions. No LLM summarization in the daemon.

## Generation and staged integration

Keep plan contracts separate until the cutover step. During steps 002–014 keep existing server compiling and develop new core under src/v1/ with a separate fresh test database. There are no preview HTTP endpoints. Both versions must never open one database. Step 015 performs the complete authenticated REST cutover. After core semantics land, switch the existing generated server contract atomically and remove MVP routes/types/tests in the same cutover batch. Update the generator operation whitelist. UI cutover is similarly one integration step after isolated screens work with MSW.

Generator incompatibilities must be documented with a minimal fixture and resolved in contract/configuration, never by hand-editing generated output. Preserve generator version 0.16.0 unless a reproduced incompatibility forces a separately justified update. See validation-report.md for actual planning-generation checks.

## Streaming route exception

+getEvents, exportProject and importProject remain in OpenAPI but are excluded from generated Axum handler whitelist. Implement them as handwritten Axum Body/stream routes with the same authentication/errors/schema conformance. Normal generated validation has a 2 MiB body cap; import must use the separate authenticated staged streaming parser. Shared client/CLI export/import copy byte streams rather than deserialize a whole ExportDocument into memory. Typed record models still come from the same contract. Other 67 operations use generated handlers.
