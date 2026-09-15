# Product scope and requirements

## Stable v1, not a Jira clone

Shepherd stores and explains project work across independent human and agent sessions. It answers what can be worked on, why other work is gated, what was decided, and what a later worker needs. It never launches agents. The owner runs one local daemon and browser; multiple registered agents access that same daemon.

| ID | Required behavior |
|---|---|
| HIER-01 | Project → Goal → Epic → Task; exactly one owner at every level. |
| HIER-02 | Goal has title/description and derived epic counts, no execution gates. |
| DEP-01 | Epic dependencies stay inside one goal; task dependencies stay inside one epic; reject cycles. |
| FLOW-01 | Separate lifecycle, task phase, manual blocking and computed eligibility. |
| FLOW-02 | Accepted work can plan early; execution waits for all dependencies and plan approval. |
| FLOW-03 | Epic auto-completes from required completed tasks; cancellation/waiver are explicit. |
| CLAIM-01 | One active claim per task; bounded leases; stale reports cannot mutate work. |
| REVIEW-01 | Independent human/agent/none plan and work review policies. |
| REVIEW-02 | Review exact immutable revisions; distinct producing/reviewing agent identities. |
| CONTENT-01 | Versioned Markdown and links, persistent handoff context. |
| AUTH-01 | Server-enforced owner/agent permissions; owner controls gates and credentials. |
| DATA-01 | Atomic commands, repeatable retries, optimistic edits, durable audit events. |
| UI-01 | Hierarchy navigation, scoped dependency graphs, count-based progress. |
| API-01 | REST, CLI, MCP expose the same authoritative semantics. |
| OPS-01 | Fresh v1 storage, backups, recovery, installable macOS/Linux artifacts. |

No sprints, scheduling, time tracking, weighted progress, portfolios, external issue synchronization, binary uploads, hosted tenancy, remote agent runners, recursive epics or subtasks. Filtering existing lists by documented fields is in scope; a separate search engine and priorities are not.

## Defaults

Project settings: proposal_gate=true, planning_required=false, plan_review=human, work_review=human. Task creation copies the planning/review defaults. Owner may override per task; agent may omit or request stricter requirements but cannot lower effective defaults. Policy strictness: none < agent < human. Project default edits do not rewrite tasks. Accepted status is determined by authenticated actor and proposal gate, never a caller-supplied status.

Types: code, research, design, documentation, test, other. Each project receives these registry entries; owner can add custom keys with the same lifecycle. Refactor is code; question is research or other; review is a phase, not a built-in task type. Existing code need not preserve old type names or APIs.

## Success

Run the reference Space Game across separate agents: prepare a plan before epic prerequisites complete, independently approve it, execute it after unlock, review the result, observe counts and downstream eligibility update live. Then prove recovery from lost responses, revoked leases, restart and backup/restore. Release UI, REST, CLI, MCP and agent guide together.
