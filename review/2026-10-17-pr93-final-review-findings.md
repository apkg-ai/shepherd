# PR #93 final review — finding provenance and disposition

Final max-effort review of `v1-011-documents-and-submissions` (step 011). Verdict: no blocking correctness regression; 2 findings become permanent at merge, 13 are hardening/cleanup. This file records where each finding's code came from and what we decide to do with it.

**Provenance summary: all 15 findings are against code introduced in PR #93.** None are latent defects shipped by earlier PRs. Two findings (12, 13) flag duplication whose other half pre-exists in steps 009/010 code — PR #93 added new copies beside it, and the clean fix touches the older files. One finding (8) is a divergence between PR #93's owner-approved behavior and the pre-existing plan/04 spec table.

| # | Finding | Where | Introduced by | Severity / class |
|---|---|---|---|---|
| 1 | `link_is_safe` misses U+061C, U+180E, TAG block (E0020–E007F) and other Cf chars | `model/content.rs` | **PR #93** (validator new in 4f620b6; invisible-char handling 3b78821) | Merge-permanent: security hardening |
| 2 | `document_revisions_page` index is dead (cycle 3 switched the sort to `number`; the UNIQUE autoindex serves the real query) and the EXPLAIN test pins a phantom query | migration `20261012000001`, `schema.sql`, `connect.rs`, acceptance test | **PR #93** (index 26ffe34, orphaned by 8b0c033) | Merge-permanent: migrations immutable after merge |
| 3 | Report-time plan-kind recheck deleted; invariant rests on `selectTaskPlan` being the only column writer | `commands/reports.rs` | **PR #93** (deletion in 26ffe34 — the check existed on main via PR #92) | Defense-in-depth for step-012's future writer |
| 4 | Revision bounds asymmetric: write path unchecked at i32::MAX, `Document.revision` serde lacks the upper bound | `content/documents.rs`, `model/content.rs` | **PR #93** | Theoretical (2^31 appends); cheap write-side cap |
| 5 | `owner_filter_exists` 404 gate has zero rejection-path coverage | `content/mod.rs`, tests | **PR #93** (cycle 3, 8b0c033) | Test gap on claimed behavior |
| 6 | Missing-revision error picks its id from a HashSet — nondeterministic; main reported first-in-request-order | `content/mod.rs` | **PR #93** (dedup fix 26ffe34; regression vs PR #92's validator) | Determinism/debuggability |
| 7 | `selected_plan_revision` uses `fetch_one` → missing task is a 500, not 404 | `content/mod.rs` | **PR #93** | Unreachable today; self-arming for step 012/015 |
| 8 | No-op re-selection contradicts plan/04's select-plan row ("acceptance cleared", "task.changed"), never amended | `content/selection.rs` vs `plan/04` | **PR #93 behavior** (owner decision 2026-10-12) vs **pre-existing spec** (PR #56) | Spec amendment needed before step 015 consumes plan/04 |
| 9 | Revisions cursor carries unused `created_at`, pays an extra id→number SELECT, dead error branch | `content/reads.rs` | **PR #93** (cycle 3) | Efficiency/shape |
| 10 | `task_owned_revision_kinds` discards the id→kind pairs; plan check correct only for length-1 slices | `content/mod.rs` | **PR #93** (cycle 3 merge) | Future-caller hazard |
| 11 | Revision increment derived three independent ways in one command | `content/documents.rs` | **PR #93** | Invariant fragility |
| 12 | Fabricated `CommandContext` (nil idempotency key) for `append_events` — copies 5 and 6 of the pattern | `content/documents.rs` + pre-existing `claims.rs`/`reports.rs` | **Pattern from PRs #91/#92 (steps 009/010)**; PR #93 added two more copies | Cross-PR cleanup: constructor touches older files |
| 13 | `typed_id_serde!` macro is a second typed-id serde mechanism parallel to claim.rs's hand-written modules | `model/content.rs` + pre-existing `model/claim.rs` | **PR #93 created the split** (c2edc22); claim.rs modules from PR #91 (step 009) | Cross-PR cleanup: unifying touches older files |
| 14 | `documents_filter` fingerprint omits absent keys, diverging from the `filter_token` scheme | `content/mod.rs` vs pre-existing `queries/hierarchy.rs` | **PR #93** (divergence is new; scheme pre-exists) | Consistency |
| 15 | Dated "owner decision 2026-10-12" comments are history notes (CLAUDE.md comments rule) | `model/content.rs`, `content/selection.rs` | **PR #93** | Comment-rule violation |

Refuted by the reviewer (not findings): authenticate-after-membership in the create commands (the idempotent wrapper authenticates first); epic-terminal owner-scope gap (epic-terminal transitions force child tasks terminal atomically).

## Disposition

All 15 findings fixed in the fourth review cycle (one commit on the branch), none deferred:

1. `link_is_safe` is an ASCII-graphic allowlist (RFC 3986 URLs are ASCII), subsuming every invisible/control class; reject matrix extended with U+061C, U+180E, a TAG char and a non-ASCII IRI.
2. `document_revisions_page` dropped from the migration, `plan/contracts/schema.sql` and the connect.rs expected-index test; the EXPLAIN pin asserts the real number-ordered query against the UNIQUE autoindex. **Note:** the migration file changed on the branch — a dev database that applied the two-index version needs `DROP INDEX document_revisions_page; DELETE FROM _sqlx_migrations WHERE version = 20261012000001;` then a restart (or a fresh database).
3. Report-time plan-kind recheck restored for free via the cited-kinds map (`cited_kinds.get(&selected) == Plan`, `invalid_state` otherwise).
4. Write-side cap: appends past revision 2147483647 fail `invalid_state` before writing; `Document.revision` serde now enforces the same int32 bound.
5. `unknown_owner_filter_targets_are_not_found` covers all three owner-kind branches, the kind-less triple probe and the happy paths.
6. Missing-revision message picks the first missing id in request order.
7. `selected_plan_revision` uses `fetch_optional` → missing task is `NotFound`.
8. plan/04's select-plan row now records the no-op re-selection clause.
9. The revisions-list number seek is an inlined subquery (no extra round-trip, no dead branch; the cursor MAC makes a miss unreachable).
10. `task_owned_revision_kinds` returns the id→kind map; the plan gate looks up the exact id.
11. One `next` binding drives the revision INSERT, the absolute UPDATE and the event.
12. `CommandContext::for_events` replaces all six fabrication sites (documents ×2, claims ×3, reports ×1).
13. `typed_id_serde!` hoisted to model/mod.rs beside `typed_uuid!`; claim.rs's hand-written `serde_task_id`/`serde_actor_id` are macro invocations; `serde_number` shrank to a `deserialize_with` fn.
14. `documents_filter` uses the `filter_token` none/some fingerprint scheme.
15. Dated owner-decision comment trailers removed.

Post-fix gates: 19 suites green (327 lib tests; 20 + 24 + 11 step-touched integration), clippy/fmt clean, coverage unit 96.9% / integration 93.1% / union 96.8%, `plan/validate.py` PASS. Merge decision: ready.
