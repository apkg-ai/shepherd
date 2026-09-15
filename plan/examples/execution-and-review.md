# Execution and completed-work review

Continue the accepted targeting plan example. Executor C has an execute lease pinning plan revision V.

1. C performs external work; Shepherd does not run code. C renews within TTL and creates a task finding document describing files changed, deterministic targeting tests and commit link. Let immutable output revision be O.
2. C reports succeeded with document_revision_ids:[O], summary and links. Response 201 Session; task active/work_review, submission W pending with policy human. Epic remains active; successor Combat integration stays unavailable.
3. C attempting reviewSubmission(W) -> 403 forbidden/reviewer_policy. Owner loads W and O. Reject with nonempty reason “Cover equal-distance case” -> 201 Review; task returns active/execution, V remains selected and accepted.
4. C reacquires a new execution claim, adds revision O2 with updated evidence and reports success. Old W is rejected and still readable; new W2 pending.
5. Owner approves W2 -> task done/complete. If this was Targeting's last required task and its dependencies are done, Targeting becomes done in the same commit. Combat integration still waits for Damage too (AND join).
6. Done task cannot be reopened/edited/claimed. Create a follow-up task in another open epic for later defects. Supplemental note documents may be appended while unarchived.

With work_review=none, step 2 instead creates accepted submission and done task atomically, with no fabricated human Review. With work_review=agent, independent agent review claim replaces owner decision. Each policy is covered by acceptance tests.
