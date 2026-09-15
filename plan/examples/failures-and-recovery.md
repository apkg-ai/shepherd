# Failures and recovery expectations

| Setup / action | Exact expected result |
|---|---|
| Two agents claim same eligible task/revision simultaneously | One 201 grant; other 409 not_eligible or 412 revision_conflict; one active DB claim. |
| Report commits but response connection drops | Retry same path/body/key returns original Session; one attempt/submission/event group. |
| Same idempotency key with changed summary | 409 idempotency_conflict; no new rows. |
| Clock reaches expires_at exactly, then report | 409 lease_invalid; no report/session; next eligible worker may claim. |
| Owner blocks epic during planning | All descendant claims revoked; draft retained; renewal/report fails; no external process is magically stopped. |
| Unblock after dependency completes | Current gates recomputed; prior lease never restored. |
| Reviewer's lease expires | Pending submission retained, distinct reviewer may claim; stale approval fails. |
| Producer withdraws while reviewer holds claim | Submission withdrawn, review claim revoked; late review cannot approve it. |
| Owner cancels required task | Epic does not auto-complete; task dependency stays unmet. |
| Owner waives cancelled task | Excluded from epic required-work test, not marked done and not a satisfied task prerequisite. |
| Empty or all-waived epic | No automatic completion; owner explicitly complete with reason and prerequisites done. |
| Add dependency between epics in different goals / tasks in different epics | 409 scope_mismatch; no edge inserted. |
| Add opposing edges concurrently | At most one inserted; dependency_cycle for other. |
| Add task races last-task epic completion | Serialized: task created before completion keeps epic open, or terminal epic rejects creation. |
| Replace selected plan during execute claim | 409 active_work; pinned plan unchanged. |
| Browser saves stale document revision | 412; local editor preserved; no overwritten revision. |
| Process killed at report write failpoint | Restart sees entire report committed or none; no released lease with missing report caused by partial command. |
| SSE cursor predates replay window / is ahead after restore | resync_required then close; client full refetch and reconnect at given cursor. |
| Invalid portable import references foreign task/duplicate ID/cycle | 422 invalid_import or 409 dependency_cycle; no new project remains. |
| Restore backup with bad checksum | Fail before replacing files; original installation intact. |
| Agent token revoked after successful command | Replay still returns 401 because authentication precedes idempotency lookup. |

Tests inject failure at each documented transaction stage; do not simulate this merely by mocking a returned error before writes. Snapshot export must be exercised concurrently with real writers. Use file-backed SQLite for races and restart tests.
