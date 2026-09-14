# Domain and persistence model

## Canonical field definitions

[OpenAPI schemas](contracts/openapi.yaml) specify every wire field, enum, bound and optional input. [SQLite baseline](contracts/schema.sql) specifies persisted columns, constraints and indexes. Read both: computed response fields are not stored. Rust structs mirror these with typed UUID newtypes and chrono UTC values; enums implement serde snake_case and explicit SQLite string conversion. No HTTP types in core.

Every mutable resource has revision starting at 1 and updated_at. Increment once per command that changes that resource, including derived phase/status changes; read-only eligibility changes need no stored revision, but a dependency mutation increments the affected tasks/epics to invalidate stale edits. Immutable rows have created_at only. No caller-controlled created_at, revision or actor IDs.

### Ownership (HIER-01, DEP-01)

Project owns goals and a task-type registry. Goal owns epics. Epic owns tasks. Ownership is fixed after creation in v1: no move endpoint, no parent patch field. To move planned work, create replacement and cancel old work with reason. Never duplicate completed history into an active task. Project ID is denormalized on descendants for querying and checked by composite foreign keys. Goal IDs are not added to tasks; derive through epic.

Dependencies use two tables, epic_dependencies and task_dependencies, despite one REST collection. Both have dependent_id and prerequisite_id and unique pairs. Validate same goal/same epic, non-self and acyclicity inside the serialized write transaction. The SQL foreign keys enforce existence; core enforces scope and cycle rules. No dependency mutation is allowed if affected dependent work is terminal, actively claimed, or has a pending review; for an epic, this includes any descendant task. Adding a prerequisite does not reopen or rewrite a completed prerequisite. Human removes links; agents may add valid links to accepted unclaimed work, never remove gates.

### Responses and absence

bounded arrays encode optional singular references: selected_plan_revision_ids and accepted_plan_submission_ids contain zero or one UUID; blocks, waivers, cancellations and archives contain zero or one record. Cancellation and archive actor/reason/time are durable and exported; clearing a block retains its prior reason in the event history. DB uses nullable singular columns, converted explicitly. All other collections return [] when empty. next_cursor alone is omitted at the end of pagination. Do not infer absence from empty strings; content strings may legitimately be empty.

### Goal and progress

Counts include archived descendants (archive is visibility only). Task counts total includes all tasks; done counts only done; cancelled counts all cancelled; waived counts cancelled with waiver. UI shows “3 / 5 tasks done · 1 cancelled · 1 waived”. Completion threshold is all non-waived tasks done, with at least one non-waived task. No denominator silently removes cancelled work. Goal completed = epic total > 0 AND every epic done; project displays epic counts across goals, not a separate project lifecycle. Empty goals show 0 / 0 and completed=false. Goals have no block/approve/cancel commands.

### Documents (CONTENT-01)

Document owner is project, epic or task in the same project. Kinds are brief, plan, finding, handoff, note, decision. Project documents replace generic project knowledge storage; links are revision/session arrays, not fetched by Shepherd. First document creation writes revision 1 atomically. Subsequent revisions append only and update latest_revision_id plus document revision. Identity, document kind/owner/title are immutable in v1; create another document to change them. Referenced content can never be overwritten or deleted.

Task plan selection must reference a plan document on that same task. No plan selection on planning_required=false (422); enabling planning first needs owner policy update and unclaimed/non-review work. Selecting a different revision clears accepted_plan_submission_id and returns phase to planning. New revisions of a selected plan are not automatically selected. During execute/review claims or pending submission, reject new revisions of the selected plan with active_work; unselected notes remain appendable. During a plan claim, claimant may append/select its plan; revision updates preserve that claim and context snapshot. Claims pin plan revision at execute acquisition.

Epics/project docs are contextual notes: editing them does not retroactively revoke accepted task plans. Human must explicitly block affected tasks when scope changes. Submitted plans capture the task revision/context at preparation; immutable output plus links allow human assessment of stale assumptions.

### Tasks and types

Task policies are persisted snapshots, not dynamic inheritance. Edits to title/description/type/planning or review requirements require owner or allowed writer, no active claim/pending review, and nonterminal task; changing description or planning/review requirements invalidates plan acceptance and resets phase to planning if planning_required else execution. Title/type changes alone retain acceptance. Agent cannot lower policy or change planning_required true to false. Human can explicitly lower them only while unclaimed and not in review. Registry keys are immutable, unique per project; archived types remain valid on existing tasks but cannot be assigned on create/edit.

### Historical identity and portability

Actor is the registered principal (one owner; agents separately registered). Agent model, harness and session labels belong in client logs/session summary if useful; they never authenticate. Actor IDs on historical objects cannot be changed. Revoking an actor revokes all its active claims but preserves reports/reviews. Imported actors become revoked historical records and can never authenticate or acquire claims. A newly registered agent always receives a new actor ID.

## Mutability and archiving

Only owner archives. Task/epic must be terminal, with no claims or pending reviews. Goal/project must contain only terminal work; empty is allowed. Archive is one-way in v1, rejects further domain mutations beneath archived scope, and hides records from ordinary discovery. Read by ID/history/export remains available. No hard delete API exists. Cancellation is terminal; note documents can still be appended to terminal unarchived entities, but plans, submissions and completed outputs cannot be replaced. Archive never deletes dependencies or changes counts/readiness.
