# Early plan → independent review → later executor

Reference: Space Game / Playable combat prototype / Targeting / Implement targeting. Combat foundation is not done. Task requires planning, agent plan review, human work review. All proposals are accepted by owner first.

1. Planner A calls listWork(phase=plan). Targeting task appears even though Combat foundation is open. listWork(phase=execute) does not include it.
2. getTask returns revision R. claimTask body {"phase":"plan","ttl_seconds":300}, If-Match "R", new idempotency UUID -> 201 ClaimGrant. Task active/planning; epic remains open. Save lease.
3. createDocument owner task/kind plan with Markdown “Choose nearest valid target; deterministic tie-breaking; test no candidates”. Read latest_revision_id as V. selectTaskPlan {document_revision_id:V} with latest task revision -> 200 Task. A planning claimant may save/select this plan without losing claim.
4. reportClaim with succeeded, summary, document_revision_ids:[V], links:[] -> 201 Session. Claim reported; task active/plan_review; one pending plan submission S with producer A and policy agent.
5. Planner A attempting claim review on S -> 403 producer_cannot_review. Reviewer B claims review with submission_id:S and current task revision -> 201 grant.
6. B reads getSubmission and V exactly, then reviewSubmission with decision approve, reason “Cases sufficient”, claim_id and lease, If-Match S.revision -> 201 Review. Task active/execution and accepted_plan_submission_ids:[S]. It is still execution-ineligible because predecessor epic is open.
7. Owner/agents finish Combat foundation through required reviews. Cascade marks predecessor done. Targeting task appears in execute work list.
8. Executor C claims execution; ClaimGrant.context.selected_plan[0].id equals V. Epic becomes active. A newer unselected note must not replace V. This is the cross-session handoff acceptance assertion.

Every mutation uses a different key for a different intent, same key for a retry. The companion workflow.py captures response IDs and runs this scenario with real owner/planner/reviewer/executor token files after implementation.
