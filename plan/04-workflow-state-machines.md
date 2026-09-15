# Workflow state machines

## Persisted dimensions (FLOW-01)

Task status: proposed, open, active, done, cancelled. Task phase: planning, plan_review, execution, work_review, complete. Epic status uses the same spelling but different transitions: proposed, open, active, done, cancelled. Explicit block is orthogonal. Eligibility is derived per request/command and actor, never directly patched.

Task initial phase = planning if planning_required, otherwise execution. Human creation is open; agent creation is proposed iff proposal_gate. An open task becomes active on its first plan/execute claim and remains active across failed attempts. Task done/cancelled implies phase=complete. Epic becomes active on first descendant execute claim, not early planning. Epic can complete directly from open if required work was explicitly satisfied through valid commands. No general status PATCH.

## Eligibility algorithm (FLOW-02)

Evaluate in this order and return all relevant reasons in the listed deterministic order, with blocking resource IDs. Terminal/archived exits early. Return can_plan/can_execute/can_review independently; reasons describe current phase plus execution dependencies during early planning. allowed_actions additionally applies actor permissions and is authoritative for buttons.

```text
base = entity not archived, no archived owner, task and epic nonterminal
accepted = task.status != proposed AND epic.status != proposed
unblocked = no task.block AND no epic.block
unclaimed = no active unexpired claim (expiry compared to transaction's now)
deps_done = every task prerequisite done AND every epic prerequisite done
plan_ok = !planning_required OR (selected plan exists AND
          accepted plan submission references that revision)
can_plan = base AND accepted AND unblocked AND unclaimed AND phase=planning
can_execute = base AND accepted AND unblocked AND unclaimed AND
              phase=execution AND deps_done AND plan_ok
can_review = base AND accepted AND unblocked AND unclaimed AND
             pending submission exists AND phase in {plan_review,work_review}
             AND caller matches policy AND caller != producer for agent review
```

can_review means eligibility to acquire the review phase, not eligibility to submit a decision after acquiring it. Once an agent holds the valid review claim, can_review=false but allowed_actions includes reviewSubmission for that claimant and pending submission. Human review has no claim and uses can_review directly. Renewal/release/report are likewise permitted to the current claimant even when acquisition eligibility is false.

No-review planning still creates an accepted plan submission on successful plan report. Merely uploading/selecting Markdown does not complete planning. Owner may claim plan/execute phases using owner credentials, or delegate them. Independent agent review is credential separation, not proof of independent model reasoning. Human review may approve human-produced output; only agent self-review is forbidden.

All epic prerequisites must be done (AND joins). For epiс eligibility, can_execute means it is accepted, unblocked, nonterminal and all epic prerequisites done; can_plan means accepted/unblocked/nonterminal. Epic can_review=false. Task execution evaluates both levels; epic has no claim API. Goal has no eligibility field.

## Transition table

Every row writes audit event(s) inside the same transaction. “Task changed” includes phase/revision changes; “cascade” recomputes epic completion and affected dependent eligibility until no state changes remain. Stop at proposed, blocked, cancelled or done epics. Failure is rollback with no domain events.

| Trigger | Preconditions/actor | Result | Claims and reviews | Events / error |
|---|---|---|---|---|
| create task/epic | writer; valid owner and defaults | proposed or open | no claim | created resource `.changed`; 422 invalid input |
| accept | owner; proposed; owner scope live | open; initial task phase retained | no claim; accepting epic does not bulk-accept tasks | changed + cascade; 409 wrong state |
| claim plan | eligible worker; matching task revision | task active/planning | one plan claim | task.changed, claim.changed; 409 not_eligible |
| plan report failed | claimant, valid lease | active/planning | release; failed immutable session; drafts retained | session.recorded, task.changed, claim.changed |
| plan report succeeded | claimant; selected plan included in revision list | active/plan_review or active/execution when policy none | release; immutable session + plan submission; none accepts immediately | session.recorded, submission.changed, task.changed, claim.changed |
| claim execute | execution eligible; matching task revision | active/execution; epic active | pins accepted plan revision if required | task.changed, epic.changed, claim.changed |
| execute report failed | claimant, valid lease | active/execution | release, session failed, attempt_count+1 | session.recorded, task.changed, claim.changed |
| execute report succeeded | claimant, valid lease; >=1 task-owned output revision | active/work_review or done/complete when none | release, session + work submission; attempt_count+1 | changed events + cascade |
| claim review | eligible agent, pending submission ID | phase unchanged | exclusive review claim | claim.changed; 403 producer_cannot_review |
| approve plan | owner if human policy; independent review claimant if agent | active/execution; set accepted_plan_submission_id | accepted submission, immutable Review; release review claim | review.recorded, submission.changed, task.changed, claim.changed when applicable |
| reject plan | same authorization; nonblank reason | active/planning; clear acceptance | rejected submission + Review; release review claim | same events |
| approve work | same authorization | done/complete | accepted submission + Review; release review claim | same + cascade |
| reject work | same authorization; reason | active/execution | rejected submission + Review; release claim; plan retained | same events |
| withdraw submission | owner or producer; pending | planning for plan, execution for work | withdrawn; revoke active review claim; no fabricated Review | submission.changed, task.changed, claim.changed |
| renew | matching claimant/token, active and not expired | task unchanged | expiry=now+TTL | claim.changed; 409 lease_invalid |
| release/expire | claimant/token or system time | phase unchanged | close claim, retain drafts; no Session manufactured | claim.changed; task.changed if representation changes |
| block task | owner or agent; nonterminal, reason | set block; status/phase retained | revoke claim; pending submission remains pending | task.changed, claim.changed |
| block epic | owner or agent; nonterminal, reason | set epic block | revoke every descendant claim incl planning/review | epic.changed, task.changed, claim.changed |
| unblock | owner only; block exists | clear explicit block | no claim restoration; derive current gates; pending review stays pending | changed + cascade |
| cancel task | owner; nonterminal, reason | cancelled/complete | revoke claims, withdraw pending submissions | changed events; cancelled prerequisites remain unmet |
| cancel epic | owner; nonterminal, reason | cancelled | cancel every nonterminal descendant with propagated reason; preserve done descendants | changed events; downstream still gated |
| waive task | owner; cancelled, not already waived; epic live | mark exclusion with reason | no task success | task.changed, epic.changed + cascade |
| auto-complete epic | accepted, unblocked; own deps done; >=1 nonwaived task and all such tasks done | done | epic never claimed | epic.changed + downstream cascade |
| explicit complete epic | owner; accepted/unblocked; own deps done; empty or all tasks waived | done | no claim | epic.changed + cascade |
| select plan/change requirements | permitted writer; no execute/review claim or pending review | active/open retained; phase planning if planning_required else execution; acceptance cleared | plan claimant may save/select own output | task.changed; 409 active_work otherwise |

Session timestamps are server-derived: started_at=claim.acquired_at, ended_at=transaction now. Failed report requires nonblank failure_reason; successful report persists failure_reason="" and rejects a nonempty failure_reason. Session document_revision_ids and links are always present. Successful planning includes its selected task plan; successful execution includes at least one task-owned finding/handoff revision. Reported claims close once; execution attempt_count increments for either successful or failed execute reports, never plan/review/expiry.

TTL default 300 seconds, min 30, max 900. Use injectable UTC clock in core. Claim expiration is <=now. Sweep every 5 seconds; claim/mutation operations reconcile relevant expired claims in their write transaction. GET computes expired claims as inactive even before sweep; never requires a cleanup mutation for correctness. Task has at most one active persisted claim, including review. Lease secrets are random and stored as hashes; status checks and lease comparison occur after write serialization.

## Structural edits and completion (FLOW-03)

Creating a task under a done/cancelled/archived epic is 409 terminal. New task under an open/active epic is allowed even if siblings work; transaction ordering resolves the race with final completion: if epic completes first, creation fails; if creation wins, it becomes required. Do not auto-complete on initial empty epic creation. An explicitly blocked epic cannot complete even if children finished before its block. Unblocking re-evaluates children. Adding an epic dependency rejects when any task in its dependent epic has an active claim/pending submission; changing a task dependency checks that dependent task. Removing links follows the same guard. No backward propagation that reopens completed work.

## Failure precedence

Authenticate → header/body syntax → idempotent replay → route membership → capability → revision → lease → terminal/active-work → eligibility → content validity. Permission errors do not leak another project's object. A command whose idempotent successful result exists returns that result before checking its now-obsolete revision. Disallowed triggers return 409; expired claim report is lease_invalid even if current task phase changed. Human policy enforcement always returns 403, never a retryable gate.
