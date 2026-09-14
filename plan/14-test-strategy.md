# Verification strategy and requirement coverage

## Principles

Preserve the Rust, Hurl, Vitest/MSW, Playwright/accessibility, coverage, dependency and security test infrastructure. Step 000 removes tests whose only purpose is deleted MVP behavior, retains still-valid infrastructure/security assertions, and proves the reduced scaffold through every quality category. Later steps add stable-v1 unit, property, migration, API and interface behavior incrementally. No tests for prose wording; validate executable contracts, example schemas, link/reference integrity and the step DAG during document authoring.

## Acceptance scenarios

| Test ID | Requirement | Setup/action | Expected observation | Layer / owning step |
|---|---|---|---|---|
| A01 | HIER-01 HIER-02 | Create two goals, epics/tasks; attempt orphan and foreign ownership | Exact ownership enforced; empty goal incomplete; 404/422 as specified | Core/HTTP 003–004 |
| A02 | DEP-01 | Branch/join; self/cross-scope/opposing concurrent edges | Only same-scope acyclic graph committed | Property + file-backed concurrency 005 |
| A03 | FLOW-01 FLOW-02 | Proposed/accepted tasks; early plan before upstream completion | Plan eligible after acceptance; execute not eligible until all gates satisfied | Pure table + HTTP 005/008/009 |
| A04 | FLOW-03 | Last required task completes; task-creation race; blocked/proposed epic | Atomic cascade; serialized create vs complete; no premature completion | Core transaction 006 |
| A05 | FLOW-03 | Cancel/waive child; empty/all-waived epic; archive | Waiver excludes from epic threshold, never satisfies dependency; owner explicit empty completion; counts unchanged by archive | Core/HTTP 006 |
| A06 | AUTH-01 | Agent sends human label, changes policies, accepts human review | 403 and unchanged revisions/history | Core/HTTP 007/008/012 |
| A07 | AUTH-01 | Owner login, CSRF/Origin/Host tampering, credential revocation | Valid owner works; all invalid requests fail; revoked actor cannot replay | HTTP/security 007 |
| A08 | CLAIM-01 | Ten concurrent agents with separate DB pools claim same task | One active claim; remaining responses conflicts; unique constraint holds | File-backed integration 009 |
| A09 | CLAIM-01 | Advance injectable clock to boundary, revoke/block, renew/report | Expired/revoked lease cannot mutate; saved revisions retained | Core + restart 009/010 |
| A10 | DATA-01 | Drop successful report response; retry same key; changed payload | Same Session/attempt/submission; conflicting reuse 409 | Real HTTP 010 |
| A11 | DATA-01 | Fail after each report write and before/after commit | No partial lease/session/task/event changes; replay after commit recovers response | File-backed failpoint 010 |
| A12 | CONTENT-01 REVIEW-02 | Plan V1 approved; draft append/select V2; stale document patch | V1 immutable; selecting V2 invalidates acceptance; stale edit 412 | Core/HTTP 011 |
| A13 | REVIEW-01 REVIEW-02 | human/agent/none × plan/work, approve/reject/withdraw/expire | Exact phase and submission table; no self-review or fabricated reviewer for none | Table + HTTP 012 |
| A14 | CONTENT-01 | Planner→reviewer→executor with large prior history | Executor receives pinned accepted plan; truncation explicit | Integration 013, workflow.py |
| A15 | DATA-01 | Disconnect SSE, lag, restart, cursor ahead after restore | Replay/no missed committed state or explicit full resync | HTTP + browser 015/020 |
| A16 | UI-01 | Navigate two goals, graph branches/joins, keyboard and narrow view | Correct scoped graph, stable viewport, accessible actions/counts | Vitest/Playwright 016–020 |
| A17 | UI-01 CONTENT-01 | Dirty document save conflict; unsafe Markdown; expired login | Draft preserved; no executable HTML/external image; clean login navigation | Browser 019/020 |
| A18 | API-01 | REST/CLI/MCP same lifecycle and invalid actor/revision requests | Same resources/errors; schema inventory parity; no protocol stdout noise | Client contract/E2E 021–024 |
| A19 | OPS-01 DATA-01 | Export while writing; invalid import; full restore | Consistent snapshot; import atomic/no active identities; full restore includes keys | Integration 014 |
| A20 | OPS-01 | Kill/restart, missing key, corruption, SIGTERM | Integrity/health actionable; no silent repair; graceful lock release | Process integration 025 |
| A21 | OPS-01 | Packaged install on three target triples from unrelated cwd | Matching UI/server/CLI/MCP operate; checksum and restore pass | Release CI 026 |
| A22 | OPS-01 | 20 projects/2k tasks/10 agents on recorded runner | Measured p95 and graph target; concurrency invariants hold | Benchmark 027 |

## Property tests

Generate valid command sequences, not arbitrary invalid database rows alone. After each command assert: hierarchy ownership; scoped DAG; at most one active claim; eligibility soundness for actor/phase; terminal work cannot reopen; accepted plan references immutable selected revision; no pending submission when task done/cancelled; required tasks determine epic completion; attempts equal executed report count; idempotent replay adds no rows; failures are transactionally invisible. Include archive/block/expiry interleavings and a fixed replay seed on regression. SQL constraint tests complement domain checks for nullable composite fields and JSON references.

## Commands and prerequisites

From repository root activate Node **26** as required by .nvmrc. The documentation authoring machine currently has Node 24 only; plan schema validation can run there, but full UI/toolchain acceptance requires Node 26. Do not lower the repository pin to match a local shell. Rust 1.98.1 and generator 0.16.0 match existing pins. Hurl 8.0.1 and Playwright browser installation are needed for E2E.

```sh
# Root: documentation checks, do not modify application.
python3 plan/validate.py
node plan/validate-contracts.cjs
openapi-to-rust generate plan/contracts/openapi.yaml --types-only --dry-run --json
python3 plan/check-generation.py

# Root: future implementation contract generation/checks (writes generated files).
scripts/regen-generated.sh
npm run lint:specs
npm run generate:api --prefix ui

# core/: future Rust checks.
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo test --package shepherd-core --test proptest_invariants -- --test-threads=1
cargo test --package shepherd-core --test migration_tests
cargo test --package shepherd-server --test api -- contract

# Root: future browser/HTTP checks.
npm run lint --prefix ui
npm run fmt:check --prefix ui
npm run typecheck --prefix ui
npm run test:unit:cov --prefix ui
npm run build --prefix ui
scripts/hurl-e2e.sh
scripts/playwright-e2e.sh
scripts/smoke.sh
```

Retain test target names when they remain meaningful, or update commands and CI together when step 000 removes an obsolete target. Never report a filtered command's zero tests as passing coverage. Use new `v1_*` test files during core development and run `cargo test --workspace` to include them. Existing `quality-gates.yaml` coverage collection and `scripts/coverage-report.mjs` thresholds remain mandatory at release. Fuzz remains a later enhancement; deterministic race/recovery/property cases are v1 release gates.

## Planning validation versus application validation

[validation-report.md](validation-report.md) records only checks actually performed on this handbook. Its contract/generator success does not prove Rust server implementation, new dependencies, UI behavior or release performance. All implementation-step checkboxes start unchecked. The final live workflow script creates sample data and must only run against a disposable implemented v1 daemon.
