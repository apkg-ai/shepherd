# REST contract and command catalog

## Common protocol (API-01)

The exact paths, methods, schemas and operation IDs are in [OpenAPI](contracts/openapi.yaml). The table below is exhaustive. Base URL is http://127.0.0.1:7437. JSON request/response bodies; errors use application/problem+json. SSE is the single streaming exception. UUIDv7 resource IDs, UTC RFC3339 timestamps, positive integer resource revisions starting at 1. Whitespace-only titles/reasons are rejected after trimming. Identifiers in a route must resolve to its project; return 404 for mismatches.

All mutation responses reflect committed state. If-Match is the quoted decimal revision of the route resource (task for claim/select-plan, submission for review/withdraw, dependency for deletion). Missing required header -> 428; malformed -> 400; mismatch -> 412. New dependency creation has no existing route revision: take the project write lock, reload both ends, and validate them together. GET resource responses carry ETag with the same quoted revision. Lease renew/report/release use the lease token and actor, not a possibly obsolete task revision.

Every domain mutation needs a UUID Idempotency-Key. Exceptions are credential issuance, browser login/logout, whose one-time secrets must not be stored as replayable plaintext. Replay scope is actor ID + key; stored method, normalized path and canonical parsed JSON must match or return 409 idempotency_conflict. Retain domain command results for 7 days. Authenticate before replay lookup; replay precedes state/revision/lease validation. A successful claim replay returns the original lease even if later expired; client checks expires_at. Encrypt persisted claim response secrets with a local 0600 daemon key (see security document). Never blindly replay expired credentials or permit revoked actors.

List order is created_at ascending, then UUID ascending, except revisions/history/sessions/submissions which are newest first. Cursor is base64url JSON containing sort tuple, endpoint and normalized filter fingerprint; tampering/mismatch is 422 invalid_cursor. Default 50, max 200. Omit next_cursor when exhausted. Filters combine with AND. Lists exclude archived entities unless include_archived=true (common optional query parameter specified for entity lists). Claims default to active=false meaning include all; active=true filters unexpired active rows. Work list includes only currently eligible tasks for caller and requested phase, oldest first, and is advisory: claiming rechecks eligibility. No automatic task allocation or reservation on GET.

Maximum normal body 2 MiB; import is an authenticated owner-only streaming exception with no total byte/record count cap, Markdown body 65,536 Unicode characters plus overall byte cap. HTTP(S) artifact links only; reject credentials in URLs and javascript/data/file schemes. Validation is identical in core and HTTP. Bad JSON is 400, unsupported content type 415, oversized body 413, semantic input failures 422. Preserve these real statuses instead of the MVP blanket 422 remapping.

## Errors (API-02)

| HTTP | code | Client behavior |
|---|---|---|
| 400 | malformed_request | Fix JSON/header syntax; no retry. |
| 401 | unauthenticated | Renew human session or configure token. |
| 403 | forbidden, reviewer_policy, producer_cannot_review | Do not retry as another claimed role. |
| 404 | not_found | Refresh parent navigation. |
| 408 | import_timeout | Restart import; the staging database was deleted and no project was committed. |
| 409 | not_eligible, active_work, terminal, dependency_cycle, scope_mismatch, lease_invalid, submission_superseded, idempotency_conflict | Refetch and explain; do not repeat with a new key automatically. |
| 412 | revision_conflict | Preserve local form, refetch, offer explicit comparison/resubmit. |
| 413 | payload_too_large | Reduce content. |
| 415 | unsupported_media_type | Send application/json. |
| 422 | validation_error, invalid_cursor, invalid_import | Show field errors. |
| 428 | precondition_required | Supply required revision/idempotency/lease header. |
| 500 | internal_error | Unknown outcome: retry same key only. |
| 507 | insufficient_storage | Free staging disk space; no import committed. |
| 503 | storage_busy, integrity_failure | Busy may retry same key; integrity failure needs owner diagnostics. |

Problem type is urn:shepherd:error:<code>. request_id is safe correlation data; never include SQL, tokens or filesystem secrets. Client-side transport failure has unknown outcome; preserve idempotency key for user retry. Retry at 250/500/1000 ms only for transport errors and storage_busy, max three retries; reads may retry without keys.

## Operation catalog

Roles are refined in [security](12-security-and-local-identity.md); generic writer never grants policy changes. Create returns 201, other commands 200, deletion commands return {"ok":true}; browser login returns 201. Read responses and body definitions are exact in the contract. All commands are audited except login/logout (security logs with no content).

| Operation | Method and path | Input → output | Required actor | Revision / lease |
|---|---|---|---|---|
| `getHealth` | `GET /health` | — → Health | public | — / — |
| `createBrowserSession` | `POST /api/v1/session` | SessionLogin → BrowserSession | public | — / — |
| `getBrowserSession` | `GET /api/v1/session` | — → BrowserSession | reader | — / — |
| `deleteBrowserSession` | `DELETE /api/v1/session` | — → Ack | human | — / — |
| `listAgents` | `GET /api/v1/agents` | — → PageActor | human | — / — |
| `createAgent` | `POST /api/v1/agents` | AgentCreate → AgentToken | human | — / — |
| `revokeAgent` | `POST /api/v1/agents/{agent_id}/revoke` | ReasonInput → Ack | human | — / — |
| `listProjects` | `GET /api/v1/projects` | — → PageProject | reader | — / — |
| `createProject` | `POST /api/v1/projects` | ProjectCreate → Project | human | — / — |
| `getProject` | `GET /api/v1/projects/{project_id}` | — → Project | reader | — / — |
| `updateProject` | `PATCH /api/v1/projects/{project_id}` | ProjectPatch → Project | human | revision / — |
| `archiveProject` | `POST /api/v1/projects/{project_id}/archive` | ReasonInput → Project | human | revision / — |
| `listGoals` | `GET /api/v1/projects/{project_id}/goals` | — → PageGoal | reader | — / — |
| `createGoal` | `POST /api/v1/projects/{project_id}/goals` | GoalCreate → Goal | human | — / — |
| `getGoal` | `GET /api/v1/projects/{project_id}/goals/{goal_id}` | — → Goal | reader | — / — |
| `updateGoal` | `PATCH /api/v1/projects/{project_id}/goals/{goal_id}` | TextPatch → Goal | human | revision / — |
| `archiveGoal` | `POST /api/v1/projects/{project_id}/goals/{goal_id}/archive` | ReasonInput → Goal | human | revision / — |
| `getGoalGraph` | `GET /api/v1/projects/{project_id}/goals/{goal_id}/graph` | — → Graph | reader | — / — |
| `listEpics` | `GET /api/v1/projects/{project_id}/epics` | — → PageEpic | reader | — / — |
| `createEpic` | `POST /api/v1/projects/{project_id}/epics` | EpicCreate → Epic | writer | — / — |
| `getEpic` | `GET /api/v1/projects/{project_id}/epics/{epic_id}` | — → Epic | reader | — / — |
| `updateEpic` | `PATCH /api/v1/projects/{project_id}/epics/{epic_id}` | TextPatch → Epic | writer | revision / — |
| `archiveEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/archive` | ReasonInput → Epic | human | revision / — |
| `acceptEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/accept` | EmptyInput → Epic | human | revision / — |
| `blockEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/block` | ReasonInput → Epic | writer | revision / — |
| `unblockEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/unblock` | EmptyInput → Epic | human | revision / — |
| `cancelEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/cancel` | ReasonInput → Epic | human | revision / — |
| `completeEpic` | `POST /api/v1/projects/{project_id}/epics/{epic_id}/complete` | ReasonInput → Epic | human | revision / — |
| `getEpicGraph` | `GET /api/v1/projects/{project_id}/epics/{epic_id}/graph` | — → Graph | reader | — / — |
| `listTasks` | `GET /api/v1/projects/{project_id}/tasks` | — → PageTask | reader | — / — |
| `createTask` | `POST /api/v1/projects/{project_id}/tasks` | TaskCreate → Task | writer | — / — |
| `getTask` | `GET /api/v1/projects/{project_id}/tasks/{task_id}` | — → Task | reader | — / — |
| `updateTask` | `PATCH /api/v1/projects/{project_id}/tasks/{task_id}` | TaskPatch → Task | writer | revision / — |
| `archiveTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/archive` | ReasonInput → Task | human | revision / — |
| `acceptTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/accept` | EmptyInput → Task | human | revision / — |
| `blockTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/block` | ReasonInput → Task | writer | revision / — |
| `unblockTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/unblock` | EmptyInput → Task | human | revision / — |
| `cancelTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/cancel` | ReasonInput → Task | human | revision / — |
| `waiveTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/waive` | ReasonInput → Task | human | revision / — |
| `listTaskTypes` | `GET /api/v1/projects/{project_id}/task-types` | — → PageTaskType | reader | — / — |
| `createTaskType` | `POST /api/v1/projects/{project_id}/task-types` | TaskTypeCreate → TaskType | human | — / — |
| `updateTaskType` | `PATCH /api/v1/projects/{project_id}/task-types/{type_id}` | TaskTypePatch → TaskType | human | revision / — |
| `listDependencies` | `GET /api/v1/projects/{project_id}/dependencies` | — → PageDependency | reader | — / — |
| `createDependency` | `POST /api/v1/projects/{project_id}/dependencies` | DependencyCreate → Dependency | writer | — / — |
| `deleteDependency` | `DELETE /api/v1/projects/{project_id}/dependencies/{dependency_id}` | — → Ack | human | revision / — |
| `listWork` | `GET /api/v1/projects/{project_id}/work` | — → PageTask | reader | — / — |
| `getTaskContext` | `GET /api/v1/projects/{project_id}/tasks/{task_id}/context` | — → ContextBundle | reader | — / — |
| `claimTask` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/claims` | ClaimInput → ClaimGrant | worker | revision / — |
| `listClaims` | `GET /api/v1/projects/{project_id}/claims` | — → PageClaim | reader | — / — |
| `renewClaim` | `POST /api/v1/projects/{project_id}/claims/{claim_id}/renew` | RenewInput → Claim | claimant | — / lease (agent only for review) |
| `releaseClaim` | `POST /api/v1/projects/{project_id}/claims/{claim_id}/release` | EmptyInput → Claim | claimant | — / lease (agent only for review) |
| `reportClaim` | `POST /api/v1/projects/{project_id}/claims/{claim_id}/report` | ReportInput → Session | claimant | — / lease (agent only for review) |
| `listTaskSessions` | `GET /api/v1/projects/{project_id}/tasks/{task_id}/sessions` | — → PageSession | reader | — / — |
| `listDocuments` | `GET /api/v1/projects/{project_id}/documents` | — → PageDocument | reader | — / — |
| `createDocument` | `POST /api/v1/projects/{project_id}/documents` | DocumentCreate → Document | writer | — / — |
| `getDocument` | `GET /api/v1/projects/{project_id}/documents/{document_id}` | — → Document | reader | — / — |
| `listDocumentRevisions` | `GET /api/v1/projects/{project_id}/documents/{document_id}/revisions` | — → PageDocumentRevision | reader | — / — |
| `createDocumentRevision` | `POST /api/v1/projects/{project_id}/documents/{document_id}/revisions` | RevisionCreate → DocumentRevision | writer | revision / — |
| `getDocumentRevision` | `GET /api/v1/projects/{project_id}/document-revisions/{revision_id}` | — → DocumentRevision | reader | — / — |
| `selectTaskPlan` | `POST /api/v1/projects/{project_id}/tasks/{task_id}/plan` | SelectPlanInput → Task | writer | revision / — |
| `listSubmissions` | `GET /api/v1/projects/{project_id}/submissions` | — → PageSubmission | reader | — / — |
| `getSubmission` | `GET /api/v1/projects/{project_id}/submissions/{submission_id}` | — → Submission | reader | — / — |
| `withdrawSubmission` | `POST /api/v1/projects/{project_id}/submissions/{submission_id}/withdraw` | ReasonInput → Submission | producer_or_human | revision / — |
| `reviewSubmission` | `POST /api/v1/projects/{project_id}/submissions/{submission_id}/reviews` | ReviewInput → Review | reviewer | revision / lease (agent only for review) |
| `listSubmissionReviews` | `GET /api/v1/projects/{project_id}/submissions/{submission_id}/reviews` | — → PageReview | reader | — / — |
| `listHistory` | `GET /api/v1/projects/{project_id}/history` | — → PageEvent | reader | — / — |
| `exportProject` | `GET /api/v1/projects/{project_id}/export` | — → ExportDocument | human | — / — |
| `importProject` | `POST /api/v1/imports` | ExportDocument → ImportResult | human | — / — |
| `getEvents` | `GET /api/v1/projects/{project_id}/events` | — → Event | reader | — / — |

| `getPrincipal` | `GET /api/v1/principal` | — → Actor | reader | — / — |

## Streaming route exception

getEvents, exportProject and importProject remain in OpenAPI but are excluded from generated Axum handler whitelist. Implement them as handwritten Axum Body/stream routes with the same authentication/errors/schema conformance. Normal generated validation has a 2 MiB body cap; import must use the separate authenticated staged streaming parser. If import makes no progress for 30 seconds while receiving body chunks or validating records, delete the staging database, roll back, and return 408 `import_timeout`; reset this timer after each received chunk or validated record. There is no total-duration cap. Shared client/CLI export/import copy byte streams rather than deserialize a whole ExportDocument into memory. Typed record models still come from the same contract. Other 67 operations use generated handlers.
