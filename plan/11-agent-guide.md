# Agent guide for stable v1

This guide describes the target v1 contract. It is not compatible with the MVP. Shepherd stores state; you execute outside it. Do not start work merely because it appears in a graph.

## Connect and identify

Ask the owner to register an agent and supply its protected token file through your tool environment. Never read the owner's token to approve a human gate. Configure SHEPHERD_TOKEN_FILE or MCP startup --token-file; use explicit project/goal/epic IDs from list responses. Agent identity comes from credentials, not model/session labels. Planner, reviewer and executor can be separate registered agents.

## Standard work loop

1. `shepherd work list --project-id <id> --phase plan --json` (or execute/review). Empty list means no eligible work, not permission to override gates.
2. Read task/context and its returned revision. Inspect reasons, selected plan and document continuation paths. Claim using that revision, a newly chosen UUID idempotency key, requested phase and protected lease output file. Claim can lose a race; refetch on not_eligible/revision_conflict.
3. Work only after claim success. Renew every 100 seconds with default 300-second TTL. Renewal requires same actor and lease. If revoked/expired, stop committing results under that lease; preserve local notes and reacquire only after eligibility returns.
4. Save content as task documents/revisions with your findings and concrete artifact links. Planning: create/select a task-owned plan revision. Execution: create a finding/handoff output revision. Do not overwrite reviewed documents.
5. Report succeeded or failed using same actor/lease. Include document revision IDs and artifact links. A failed report needs failure_reason; success needs the required plan/output. Server releases claim and creates immutable session/submission atomically.
6. If policy requires review, wait. Reviewer uses review phase claim and exact submission ID. Human review is done by owner, never by an agent calling itself human. Independent agent review rejects same producer credential.
7. On rejection read review reason; next claim returns planning or execution as appropriate. Revise content, submit again, preserve old evidence. Done means accepted completion, not merely a successful execution report.

## Early planning

An epic waiting on its predecessor may still contain plan-eligible tasks. Preparation does not start execution or mark epic active. Plan should state assumptions and reference known dependencies. After accepted plan, execution remains unavailable until all task and epic prerequisites are done. Do not treat a reviewed plan as permission to ignore a dependency.

## Retry and handoff

Create one idempotency key per intent and save it with your local work state. A timeout may occur after commit: repeat exact method/path/body/key, not a new command. If key body mismatch returns conflict, recover original intent rather than guessing. GET after report confirms current task/submission state. Reports after lease loss are rejected even if local work succeeded; save local content and tell owner before reacquiring.

Handoff content includes objective, selected plan revision, work performed, files/commits/PR links, validation run and results, unresolved issues, and next action. Avoid full transcripts by default; task-owned Markdown plus links is sufficient. Context may be truncated; follow continuation_paths for needed history. No credentials in content. Block with a clear reason when an external condition prevents progress; only owner clears explicit blocks.

## Worked examples

Use [reference project](examples/reference-project.json), [planning handoff](examples/planning-handoff.md), [execution/review](examples/execution-and-review.md), and [failure/recovery](examples/failures-and-recovery.md). Concrete per-operation request/response examples are in [operation examples](examples/operations.json); replace illustrative IDs with captured server responses during live execution.
