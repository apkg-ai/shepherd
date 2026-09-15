# Shared vocabulary

| Term | Exact meaning | Do not use as synonym |
|---|---|---|
| Project | Overall effort, e.g. Space Game | Repository-only assumption |
| Goal | Outcome inside project, e.g. Playable combat prototype | Claimable work or dependency node |
| Epic | Outcome-sized container of tasks, with epic prerequisites | Task type |
| Task | Atomic schedulable unit owned by one epic | Epic, step, recursive subtask |
| Phase | Planning, plan review, execution, work review, complete | Dependency or grouping level |
| Status | Lifecycle progress: proposed/open/active/done/cancelled | Readiness or blocking |
| Eligibility | Whether this actor may acquire a specific phase now | Stored editable status |
| Block | Explicit reversible stop with actor/reason | Waiting for a prerequisite |
| Dependency | Dependent requires prerequisite to be done | Parent/child relationship |
| Claim | Exclusive task lease for plan/execute/review | Permanent assignment |
| Session | Immutable plan/execute work report | Authentication/browser session |
| Submission | Immutable plan or result set awaiting/recording acceptance | Mutable document draft |
| Review | One actor's approve/reject decision on a submission | Proposal acceptance |
| Document | Addressable Markdown artifact with immutable revisions | Arbitrary metadata blob |
| Revision | Immutable content version, or monotonic mutable-resource counter | Git commit |
| Waiver | Owner excludes a cancelled task from epic completion | Task success or satisfied dependency |
| Archive | Hide from default lists without erasing history | Delete or unblock |

Wire names use snake_case, Rust types UpperCamelCase, OpenAPI operation IDs lowerCamelCase. UI uses readable labels: plan_review = “Plan review”, open = “Not started”, active = “Started”, execution = “Execution”. Display “Waiting on …” for dependencies and “Blocked: …” for manual blocks. Never label proposed work “ready”.

Arrows in the UI point prerequisite → dependent. Dependency payload uses dependent_id + prerequisite_id explicitly; no ambiguous source/target fields.
