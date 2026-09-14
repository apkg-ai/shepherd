# MCP adapter

## Transport and dependencies

Use [official Rust SDK rmcp](https://docs.rs/rmcp/3.3.0/rmcp/) pinned in [dependencies](dependencies.md), stdio transport only. Startup: `shepherd-mcp --url http://127.0.0.1:7437 --token-file /absolute/path/agent-token`. Authenticate shared REST client and verify agent identity using a read operation plus credential validation; an owner credential is rejected by adapter startup (add shared client introspection via getPrincipal, specified in contract). Do not expose owner credentials, login or token issuance through tools.

Use negotiated SDK protocol version and tool capabilities only. No sampling, autonomous execution, resources/prompts subscriptions, MCP Tasks extension or hosted transport. STDOUT exclusively contains JSON-RPC protocol messages; diagnostic logs go STDERR. EOF cancels outstanding network calls and exits; existing claims expire rather than being falsely reported as complete.

## Tool inventory and shape

[adapter-map.json](contracts/adapter-map.json) is exact inventory; non-null mcp entries must all be exposed. Tool name is shepherd_ + snake_case(operationId). Input object contains path parameters, query parameters and optional body matching the REST schema. For commands add expected_revision if If-Match is required, idempotency_key when required, and lease_token if claimant operation; review lease conditional on agent policy is required because this adapter is agent-only. All objects additionalProperties=false. No actor kind, owner token or bearer token parameters. Lease token is a task-scoped capability and necessarily returned by claim; do not log tool arguments/results containing it.

Success returns structuredContent equal to REST response and a text JSON equivalent. Claim success includes lease_token so agent can renew/report; MCP secrets remain in client session, not audit/document storage. Tool schema uses same UUID/enums/bounds as REST. Domain/HTTP errors return isError=true and structured Problem; JSON-RPC parameter parse errors use SDK error handling. Network failure is isError with code client_transport and unknown_outcome=true; agent retries same key after reconnect. Never convert a rejected command into successful prose.

Set readOnlyHint=true for GET tools; destructiveHint=true for block and withdraw; idempotentHint=true only for reads or commands with supplied idempotency keys (retention caveat in description); openWorldHint=false because only local configured daemon is addressed. Annotations are client hints, not authorization. No automatic renew or task selection; guide tells agent to list, claim, heartbeat, report.

## Deliberate exclusions

Human-only project/goal administration, acceptance/cancellation/waivers/unblocking, type administration, dependency removal, credential/session management, import/export and SSE are absent. Owner uses UI/CLI/REST for these. Agent tools still read goals/projects, create/edit epics/tasks within policy constraints, add dependencies, block, create documents, claim plan/execute/review, report, renew/release, inspect history and review/withdraw when allowed. getHealth and getPrincipal are startup/internal client calls, not tools. These exclusions keep human gates reviewable while preserving all agent work capabilities.

## Acceptance

Spawn actual binary with temporary token file and stdio pipes. Initialize and compare tools/list against mapping (no omissions/extras). Validate each tool input schema against REST samples. Execute planning handoff using planner/reviewer processes with distinct tokens. Assert rejected human gate and self-review produce identical Problem codes to REST. Stop daemon during report: error is unknown outcome; replay same key after restart does not duplicate session. Inspect STDOUT to ensure it contains protocol frames only.
