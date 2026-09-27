# Full codebase review — 2026-09-27

Reviewed commit: `b51b0dc` (`Record step 006 review follow-ups in handoff`). Scope: tracked Rust core/server, SQLite migrations, served and planning contracts, React UI, tests, scripts, dependency manifests, and GitHub workflows. Generated code, build output, installed dependencies, and the untracked video recordings were excluded as source; generated Rust/TypeScript was exercised through the contract gate. This is a review against the repository's `AGENTS.md` and `plan/` requirements, plus directly observable security, correctness, and performance behavior; it is not a certification against an unspecified external standard.

The tree is an intermediate v1 step-006 implementation. The production daemon currently serves `/health`, the scaffold OpenAPI document, and static UI files; it does not open the SQLite store or expose domain commands. Findings about domain API abuse are integration or release risks, not claims of a presently reachable unauthenticated data API. Severity is relative to completing v1: **P1** requires a boundary fix before another adapter uses the core; **P2** should be fixed before REST/domain release or performance acceptance; **P3** is a quality or assurance gap. A passing scanner does not prove absence of bugs.

## Findings

### R-01 — P1 — Public transaction API bypasses the core permission and audit boundary

[`Store::command_transaction`](../core/shepherd-core/src/storage/transaction.rs#L27) is `pub` and hands any downstream Rust crate a mutable `sqlx::Transaction`. The comment on [`Store::pool`](../core/shepherd-core/src/storage/mod.rs#L34) says downstream code must not bypass command handling, and [`domain_transaction`](../core/shepherd-core/src/commands/mod.rs#L40) is crate-private, but the public transaction method permits arbitrary SQL against `actors`, `projects`, `credentials`, `events`, and other tables. Internal test fixtures already demonstrate direct actor insertion through this route. An adapter can therefore bypass `live_actor`, owner checks, revision checks, and event creation. This is an API boundary flaw, not a current HTTP exploit. Make the raw transaction method crate-private and provide narrow bootstrap/identity operations for legitimate external callers; add an external-crate compile check proving raw SQL is unavailable.

### R-02 — P2 — Host is unchecked and CORS trusts every loopback port

[`cors_layer`](../core/shepherd-server/src/middleware.rs#L9) uses byte-prefix checks for `localhost`, `127.0.0.1`, and `[::1]` with any port, and [`router`](../core/shepherd-server/src/lib.rs#L25) has no Host validation. The live probe `curl -si -H 'Host: attacker.example:7559' -H 'Origin: http://localhost:9999' http://127.0.0.1:7559/health` returned `200 OK` and `access-control-allow-origin: http://localhost:9999`. The [security plan](../plan/12-security-and-local-identity.md#L35) requires exact Host/Origin values tied to the configured port and an explicit development exception. Current exposed data is public scaffold content; once credentials and mutations land, the broad origin policy and arbitrary Host acceptance become a local-web attack surface. Pass the configured listener address into middleware, parse and compare complete origins, reject unexpected Host values, and test negative and preflight cases.

### R-03 — P2 — Domain commands ignore their idempotency key

[`CommandContext`](../core/shepherd-core/src/commands/mod.rs#L21) carries `idempotency_key`, and the [storage layer](../core/shepherd-core/src/storage/rows.rs#L91) can save/retrieve replay records, but production command implementations never read or write that key. A retry of `create_project`, `create_goal`, or `create_task` with the same key executes again and creates another resource; a retry of a revisioned mutation encounters the new revision instead of replaying its committed response. The [REST contract](../plan/07-rest-contract.md#L9) requires replay semantics. This is scheduled work before the REST adapter, but the current core API does not provide the guarantee its context type implies. Implement authenticated, request-bound replay in the serialized transaction before exposing these commands over REST/CLI/MCP, and test duplicate requests plus conflicts with the same key and different input.

### R-04 — P2 — Every response advertises a rate limit that is never enforced

[`RateLimitHeaderService`](../core/shepherd-server/src/middleware.rs#L53) always emits `1000`, `999`, and `60` regardless of requests and has no counter or rejection path. The [server test](../core/shepherd-server/tests/api.rs#L63) only checks header presence; the served [OpenAPI response](../openapi/shepherd.yaml#L55) describes these as actual quota state and a `429` response. The [security plan](../plan/12-security-and-local-identity.md#L37) explicitly disallows broad fake rate-limit headers. Remove the advertised quota until a limiter exists, or implement and test a real quota with decreasing remaining count and `429` behavior.

### R-05 — P2 — The served OpenAPI document directs clients to the wrong transport and token format

The served [OpenAPI `servers` URL](../openapi/shepherd.yaml#L17) is `https://localhost:7437`, while [`main.rs`](../core/shepherd-server/src/main.rs#L29) serves plain HTTP and the description in the same spec says TLS is post-v1. The spec also advertises [`bearerFormat: JWT`](../openapi/shepherd.yaml#L107), whereas the [identity plan](../plan/12-security-and-local-identity.md#L7) specifies random opaque base64url tokens. A client that honors the server URL cannot connect to the current daemon; a client that validates JWT shape would reject the planned token. Correct the scaffold spec now and keep the generated client/served-spec conformance gate aligned when authentication lands.

### R-06 — P2 — Common task lists scan a project and use a temporary sort for each page

[`list_tasks`](../core/shepherd-core/src/queries/hierarchy.rs#L754) orders project-wide results by `(created_at,id)`, but the existing [`tasks_project` index](../core/shepherd-core/migrations/20260914000001_baseline.sql#L303) places `status,phase` between `project_id` and that order. On the baseline schema, `EXPLAIN QUERY PLAN SELECT id FROM tasks WHERE project_id='p' AND archived=0 ORDER BY created_at,id LIMIT 51` reports `SEARCH tasks USING INDEX tasks_project (project_id=?)` followed by `USE TEMP B-TREE FOR ORDER BY`; the work-list shape with `status IN (...)` also uses a temporary sort. A 50-item page can therefore scan every task in a large project, even if SQLite keeps only a bounded sort buffer. Add a new migration with an index matching the unfiltered page order (and benchmark the important filtered shapes with realistic distributions); do not edit the applied baseline migration.

### R-07 — P2 — A deep epic-completion chain amplifies queries and scans pending events quadratically

[`recompute`](../core/shepherd-core/src/workflow/mod.rs#L32) loads a fresh snapshot and dependent IDs for each frontier wave. A linear dependency chain produces one epic per wave, so a completion of depth `d` performs repeated multi-query snapshot loads while holding the serialized write transaction. The [`events.iter().any`](../core/shepherd-core/src/workflow/mod.rs#L64) check rescans a growing vector on each completion, adding O(`d²`) in-memory work. This is a code-path complexity finding; no large-chain latency benchmark was run. Replace the event scan with a set keyed by resource, and add a 1,000–2,000-epic chain benchmark before accepting the current cascade architecture for performance requirements.

### R-08 — P3 — CSP is absent and the built shell contains inline JavaScript

The server layers in [`router`](../core/shepherd-server/src/lib.rs#L25) set CORS and the quota headers but no Content-Security-Policy. [`ui/index.html`](../ui/index.html#L8) contains an inline theme script, which the [planned `script-src 'self'` policy](../plan/12-security-and-local-identity.md#L37) would block without a hash or moving the script to a file. The current UI has no stored user content, so this is a hardening/release integration gap rather than a demonstrated XSS exploit. Implement the planned response policy and adapt the pre-paint script without relaxing the script directive.

### R-09 — P3 — The CI fuzz job is a success-shaped placeholder

[`core-test-fuzz`](../.github/workflows/quality-gates.yaml#L588) only echoes that there are no fuzz targets and always passes. It does not exercise parsers, cursor decoding, graph mutations, or SQLite recovery. Either add bounded fuzz/property targets and a real gate, or rename/remove the job so the CI status does not imply fuzz coverage. The other test suites remain useful and passed in this review.

### R-10 — P3 — Hand-written core modules exceed the repository's size rule

[`AGENTS.md`](../AGENTS.md#L19) calls for splitting hand-written files well before a few hundred lines. Current files include `commands/hierarchy.rs` (3,672 lines; 1,355 before its test module), `workflow/eligibility.rs` (1,679; 991 production), `queries/hierarchy.rs` (1,598; 905 production), and `commands/dependencies.rs` (1,238). The [step-006 handoff](../plan/steps/006-completion-and-blocking.md#L108) already acknowledges a deferred split for hierarchy. The concern is reviewability and change isolation, not an asserted runtime fault; split by command/query responsibility while preserving public behavior and tests.

### R-11 — P3 — Dependency freshness policy is not met, although known-advisory audits are clean

[`AGENTS.md`](../AGENTS.md#L32) says to keep dependencies current. On 2026-09-27, `npm outdated --json` reported 2 root and 10 UI direct packages behind published versions; examples include `@tanstack/react-query` 5.103.1→5.104.0 and `vite` 8.3.0→8.3.1 (confirmed on the [React Query npm page](https://www.npmjs.com/package/%40tanstack/react-query?activeTab=versions) and [Vite npm page](https://www.npmjs.com/package/vite?activeTab=versions)). `cargo update --dry-run --manifest-path core/Cargo.toml` proposed 24 lockfile updates, including `thiserror` 2.0.20→2.0.21; it made no tree changes. `npm audit` for both lockfiles and `cargo audit` for 313 Rust packages reported zero advisories, so version lag is not being presented as a vulnerability. Refresh lockfiles and eligible ranges in a separate tested change; preserve the deliberate `jsonschema` and `lodash` pins noted in `AGENTS.md`.

### R-12 — P3 — Root README misstates the implementation stage

[`README.md`](../README.md#L5) says the whole project is at step 000, while the [step-006 handoff](../plan/steps/006-completion-and-blocking.md#L1) and current core implement six stages beyond the scaffold. The server still exposes only the scaffold, so the accurate description is “step-006 core with a health-only server.” The current wording makes reviewers and contributors miss the domain code that needs review. Update the state line and keep the distinction between implemented core and served API explicit.

### R-13 — P2 — A toast can expire while its dismiss button still has focus

[`ToastProvider`](../ui/src/components/Toast.tsx#L42) starts a new five-second timer on every `onMouseLeave` and `onBlurCapture`, independently of whether focus or hover still holds the toast open. For example, focus the dismiss button, hover the toast, then move the pointer away: `onMouseLeave` restarts expiry while the button remains focused, and the toast disappears five seconds later. Repeated leave/blur events can also overwrite the timer map without clearing the earlier timer. Existing [tests](../ui/src/components/components.test.tsx#L352) cover hover and focus separately, so this interaction is missed. Track hover and focus together, keep exactly one timer per toast, and add a mixed pointer/keyboard regression test.

## Additional checks and limits

| Check run during this review | Result |
|---|---|
| `cargo test --manifest-path core/Cargo.toml --workspace --all-targets` | Passed, including server API and boot tests. |
| `cargo clippy --manifest-path core/Cargo.toml --workspace --all-targets -- -D warnings`; `cargo fmt --manifest-path core/Cargo.toml --all -- --check` | Passed. A plain production `cargo build` still prints dead-code warnings for staged future features. |
| UI `node --run test:unit`, `typecheck`, `lint`, `fmt:check`, `build` | 128 unit tests passed; typecheck, formatting, and build passed. Lint exited 0 with three React fast-refresh warnings. |
| `scripts/hurl-e2e.sh`; `scripts/playwright-e2e.sh` | 2 Hurl files / 5 requests and 7 Chromium tests passed, including the current two-route axe audit. |
| `scripts/check-v1-contracts.sh --node-dir /Users/sheplu/.nvm/versions/node/v24.19.0/bin`; `python3 plan/validate.py`; root `node --run lint:spec` and `typecheck`; `shellcheck scripts/*.sh` | Passed. The contract gate compiled generated Rust and TypeScript in scratch directories and checked negative fixtures. |
| Semgrep with the CI registry rules on tracked source, scripts, and workflows | 90 tracked files scanned, 288 applicable rules, 0 findings. CI exclusions and generated files were not scanned. |
| Root/UI `npm audit --audit-level=low --json`; `cargo audit --file core/Cargo.lock` | Zero reported advisories. The RustSec database contained 1,271 advisories at scan time. |
| SQLite `EXPLAIN QUERY PLAN` on the applied baseline schema | Confirmed temporary B-tree sorts for the task-list and work-list query shapes. |
| Live loopback `curl` with unrelated Host and local Origin | Confirmed `200 OK`, accepted Origin, and static rate-limit headers. |

The local Node executable was 24.19.0 while `.nvmrc` requests 26; this review did not reproduce the exact CI Node version. The local machine was macOS arm64 rather than CI's Ubuntu arm64. OSV Scanner was not installed locally, and the combined llvm-cov coverage threshold was not regenerated; neither check is claimed as passed here. The current Hurl and Playwright suites cover the health-only server and shell, not the not-yet-served domain workflows. No dependency or product source files were changed during this review.

Before step 015 exposes domain routes, address R-01 through R-05 and R-08 together as the external boundary is wired. Fix R-13 in the UI before its toast component is used for domain outcomes. Measure R-06 and R-07 with representative projects before step 027 performance acceptance. R-09 through R-12 can be tracked as repository maintenance without treating the current green tests as evidence they are resolved.
