# CLI and shared Rust client

## Crates and interface

Add shepherd-client (library), shepherd-cli (binary named shepherd), shepherd-mcp (binary). Shared client owns generated wire structs and typed operation methods; no dependency on shepherd-core or shepherd-server. Use pinned reqwest and existing clap/serde/Tokio; see [dependency choices](dependencies.md).

Canonical complete interface is `shepherd api <operationId>`. operationId is case-sensitive from the mapping table. Every path parameter becomes a required kebab flag (project_id → --project-id); query parameters become optional kebab flags. Body is --body-file PATH or --body-file - for stdin. Refuse unknown fields through generated validation/server contract; no implicit JSON strings in positional arguments. Commands with EmptyInput send {} automatically. --if-match N converts to quoted If-Match; --idempotency-key UUID must be supplied for retryable mutations. Reads never need these flags.

Global flags: --url (default http://127.0.0.1:7437), --token-file PATH (required or SHEPHERD_TOKEN_FILE), --json, --output PATH. No --token argument. Local clients reject non-loopback URLs, redirects and proxies. Token file must not be group/world readable. --lease-file PATH reads a 0600 JSON claim grant for claimant operations and injects X-Lease-Token; --claim-id may be inferred from grant if not supplied. claimTask requires --save-lease PATH and refuses overwriting an existing file. Secret grant is written 0600; stdout prints Claim without lease token unless --output explicitly names a protected file. A credential issuance operation similarly requires --output PATH and never prints token to terminal by default.

Generic api getEvents is the only streaming command: write one Event JSON per line, retain cursor in memory, follow documented resync behavior and print resync_required to stderr. Export --output writes complete export JSON. Import body is that file. Full backup/restore/doctor are local shepherd-server maintenance commands, not REST client commands.

## Ergonomic aliases

| Alias | Canonical operation | Extra behavior |
|---|---|---|
| shepherd work list | listWork | --phase required |
| shepherd task get | getTask | --task-id required |
| shepherd task claim | claimTask | --phase becomes body.phase; --ttl default 300; optional --submission-id; save lease required |
| shepherd claim renew | renewClaim | --ttl becomes body.ttl_seconds |
| shepherd claim release | releaseClaim | sends {} |
| shepherd claim report | reportClaim | --body-file required |
| shepherd task context | getTaskContext | no hidden claim |
| shepherd document append | createDocumentRevision | --document-id, --body-file, --if-match |
| shepherd submission review | reviewSubmission | --submission-id, --body-file, --if-match; lease for agent |

Aliases call the same typed client methods; no separate semantics. Every command also exists canonically, so implementation coverage is measurable against adapter-map.json.

## Output and failure

--json: success is exactly response JSON (except secrets redacted as above); stderr carries diagnostics only. Default human output renders a compact resource/phase/gate table; do not truncate --json content. Exit codes: 0 success, 2 argument/validation 400/413/415/422/428, 3 auth 401/403, 4 not found, 5 conflict 409/412, 6 transport unknown-outcome, 7 server 500/503. Error JSON is Problem on stderr with --json; local errors use code client_transport/client_usage and no invented server request ID. Retry only per REST protocol; client does not create new keys automatically. No auto-renew daemon or background process: caller must renew every TTL/3 while working.

## Complete operation mapping

| REST operation | CLI | MCP |
|---|---|---|
| `getHealth` | `shepherd api getHealth` | Not exposed; see MCP exclusions |
| `createBrowserSession` | `shepherd api createBrowserSession` | Not exposed; see MCP exclusions |
| `getBrowserSession` | `shepherd api getBrowserSession` | Not exposed; see MCP exclusions |
| `deleteBrowserSession` | `shepherd api deleteBrowserSession` | Not exposed; see MCP exclusions |
| `listAgents` | `shepherd api listAgents` | Not exposed; see MCP exclusions |
| `createAgent` | `shepherd api createAgent` | Not exposed; see MCP exclusions |
| `revokeAgent` | `shepherd api revokeAgent` | Not exposed; see MCP exclusions |
| `listProjects` | `shepherd api listProjects` | `shepherd_list_projects` |
| `createProject` | `shepherd api createProject` | Not exposed; see MCP exclusions |
| `getProject` | `shepherd api getProject` | `shepherd_get_project` |
| `updateProject` | `shepherd api updateProject` | Not exposed; see MCP exclusions |
| `archiveProject` | `shepherd api archiveProject` | Not exposed; see MCP exclusions |
| `listGoals` | `shepherd api listGoals` | `shepherd_list_goals` |
| `createGoal` | `shepherd api createGoal` | Not exposed; see MCP exclusions |
| `getGoal` | `shepherd api getGoal` | `shepherd_get_goal` |
| `updateGoal` | `shepherd api updateGoal` | Not exposed; see MCP exclusions |
| `archiveGoal` | `shepherd api archiveGoal` | Not exposed; see MCP exclusions |
| `getGoalGraph` | `shepherd api getGoalGraph` | `shepherd_get_goal_graph` |
| `listEpics` | `shepherd api listEpics` | `shepherd_list_epics` |
| `createEpic` | `shepherd api createEpic` | `shepherd_create_epic` |
| `getEpic` | `shepherd api getEpic` | `shepherd_get_epic` |
| `updateEpic` | `shepherd api updateEpic` | `shepherd_update_epic` |
| `archiveEpic` | `shepherd api archiveEpic` | Not exposed; see MCP exclusions |
| `acceptEpic` | `shepherd api acceptEpic` | Not exposed; see MCP exclusions |
| `blockEpic` | `shepherd api blockEpic` | `shepherd_block_epic` |
| `unblockEpic` | `shepherd api unblockEpic` | Not exposed; see MCP exclusions |
| `cancelEpic` | `shepherd api cancelEpic` | Not exposed; see MCP exclusions |
| `completeEpic` | `shepherd api completeEpic` | Not exposed; see MCP exclusions |
| `getEpicGraph` | `shepherd api getEpicGraph` | `shepherd_get_epic_graph` |
| `listTasks` | `shepherd api listTasks` | `shepherd_list_tasks` |
| `createTask` | `shepherd api createTask` | `shepherd_create_task` |
| `getTask` | `shepherd api getTask` | `shepherd_get_task` |
| `updateTask` | `shepherd api updateTask` | `shepherd_update_task` |
| `archiveTask` | `shepherd api archiveTask` | Not exposed; see MCP exclusions |
| `acceptTask` | `shepherd api acceptTask` | Not exposed; see MCP exclusions |
| `blockTask` | `shepherd api blockTask` | `shepherd_block_task` |
| `unblockTask` | `shepherd api unblockTask` | Not exposed; see MCP exclusions |
| `cancelTask` | `shepherd api cancelTask` | Not exposed; see MCP exclusions |
| `waiveTask` | `shepherd api waiveTask` | Not exposed; see MCP exclusions |
| `listTaskTypes` | `shepherd api listTaskTypes` | `shepherd_list_task_types` |
| `createTaskType` | `shepherd api createTaskType` | Not exposed; see MCP exclusions |
| `updateTaskType` | `shepherd api updateTaskType` | Not exposed; see MCP exclusions |
| `listDependencies` | `shepherd api listDependencies` | `shepherd_list_dependencies` |
| `createDependency` | `shepherd api createDependency` | `shepherd_create_dependency` |
| `deleteDependency` | `shepherd api deleteDependency` | Not exposed; see MCP exclusions |
| `listWork` | `shepherd api listWork` | `shepherd_list_work` |
| `getTaskContext` | `shepherd api getTaskContext` | `shepherd_get_task_context` |
| `claimTask` | `shepherd api claimTask` | `shepherd_claim_task` |
| `listClaims` | `shepherd api listClaims` | `shepherd_list_claims` |
| `renewClaim` | `shepherd api renewClaim` | `shepherd_renew_claim` |
| `releaseClaim` | `shepherd api releaseClaim` | `shepherd_release_claim` |
| `reportClaim` | `shepherd api reportClaim` | `shepherd_report_claim` |
| `listTaskSessions` | `shepherd api listTaskSessions` | `shepherd_list_task_sessions` |
| `listDocuments` | `shepherd api listDocuments` | `shepherd_list_documents` |
| `createDocument` | `shepherd api createDocument` | `shepherd_create_document` |
| `getDocument` | `shepherd api getDocument` | `shepherd_get_document` |
| `listDocumentRevisions` | `shepherd api listDocumentRevisions` | `shepherd_list_document_revisions` |
| `createDocumentRevision` | `shepherd api createDocumentRevision` | `shepherd_create_document_revision` |
| `getDocumentRevision` | `shepherd api getDocumentRevision` | `shepherd_get_document_revision` |
| `selectTaskPlan` | `shepherd api selectTaskPlan` | `shepherd_select_task_plan` |
| `listSubmissions` | `shepherd api listSubmissions` | `shepherd_list_submissions` |
| `getSubmission` | `shepherd api getSubmission` | `shepherd_get_submission` |
| `withdrawSubmission` | `shepherd api withdrawSubmission` | `shepherd_withdraw_submission` |
| `reviewSubmission` | `shepherd api reviewSubmission` | `shepherd_review_submission` |
| `listSubmissionReviews` | `shepherd api listSubmissionReviews` | `shepherd_list_submission_reviews` |
| `listHistory` | `shepherd api listHistory` | `shepherd_list_history` |
| `exportProject` | `shepherd api exportProject` | Not exposed; see MCP exclusions |
| `importProject` | `shepherd api importProject` | Not exposed; see MCP exclusions |
| `getEvents` | `shepherd api getEvents` | Not exposed; see MCP exclusions |

| `getPrincipal` | `shepherd api getPrincipal` | Internal startup check; not a tool |

Export/import are streaming operations: --output and --body-file are mandatory, request/response bytes are copied incrementally using the shared client's streaming methods. Do not allocate an ExportDocument Vec containing an entire long-lived project. The import result ID map can also grow; stream the response to --output when importing, and print only project_id in human mode. Server emits the result map incrementally after successful commit. --json without --output streams complete JSON to stdout.
