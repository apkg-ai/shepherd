# 05 — Testing & CI

> Strong testing is a stated expectation. Eight explicit layers, each with a clear owner, wired into a
> staged CI. The invariants under test are defined in [02-domain-model.md](02-domain-model.md).

## Taxonomy

| Layer | Owns | Tooling |
|---|---|---|
| **Unit** | Domain lib: lifecycle state machine, DAG operations, lease logic, context-bundle assembly. UI: components and utilities. | `cargo test`; `vitest` + Testing Library |
| **Property-based** | The [02](02-domain-model.md) invariants under arbitrary operation sequences: dependency acyclicity, legal-only transitions, claim/lease invariants, export/import round-trip. | `proptest` |
| **Integration** | axum handlers against a real temp SQLite — full request → handler → DB → response, SSE event emission, concurrency (N parallel claims on one task → exactly one winner). | `cargo test` (spawned test server) |
| **Contract** | Every live response conforms to `openapi/shepherd.yaml`; spec drift fails CI. The spec is the coupling point for bbq/CLI/MCP — this layer is critical. | Spectral (static) + dynamic conformance in integration tests |
| **Migration** | Every `sqlx` migration applies cleanly on seeded fixture DBs; existing data survives. A long-lived local hub makes data loss the worst failure mode. | `sqlx` test harness |
| **API E2E** | The REST contract over a real socket: a booted server on a fresh DB driven by plain HTTP scripts — CRUD, lifecycle actions, relations, error shapes. Complements the in-process integration layer by exercising the real binary and wire bytes. | `hurl` (`scripts/hurl-e2e.sh`, `tests/hurl/*.hurl`) |
| **E2E** | Browser driving the real UI against a real server: register → create/approve → both graph lenses → live SSE update → review queue. | Playwright |
| **Smoke** | Seconds-fast boot sanity: server starts on a fresh DB, `/health` OK, spec served, UI index loads. Gates every PR and release artifact. | Minimal CI script |
| **Fuzz** | Untrusted input boundaries: task `metadata` JSON, graph mutation payloads, query params, import documents. | `cargo-fuzz` — short per PR, deep nightly |

**Judgment call, recorded:** for this domain, property-based tests outrank raw fuzzing — the graph and
lifecycle invariants are where logic bugs live; fuzz earns its keep on the parse/validation boundaries.

## CI stages (GitHub Actions)

| Stage | When | Runs |
|---|---|---|
| **Lint** | every PR | `cargo fmt --check`, `clippy` (deny warnings), oxlint, `tsc`, Spectral |
| **Test** | every PR | unit + property + integration + migration + contract; `vitest` |
| **API E2E** | every PR | hurl suite against a booted server (`scripts/hurl-e2e.sh`) |
| **Generated-code drift** | every PR | regenerate `src/generated` from the spec and fail on diff (`scripts/regen-generated.sh`) |
| **Smoke** | every PR | boot check (fresh DB → health → spec → UI index) |
| **E2E** | every PR | Playwright primary flow |
| **Fuzz (short)** | every PR | ~2–3 min per target |
| **Fuzz (deep)** | nightly | extended runs, all targets |

Coverage is gated in CI (`cargo-llvm-cov`, vitest coverage), scoped so each suite measures the code it
owns (unit → `shepherd-core` + `ui/src`, integration → server lib; `main.rs` is smoke-covered). Two
sticky PR reports — Core (Rust) and UI (TypeScript) — each union-merged. Thresholds on lines:
unit ≥ 95%, integration ≥ 70%, e2e ≥ 50% (enforced from S9), per-report total ≥ 92%.

The CI skeleton lands in **S1** and every subsequent session ends green — no session merges red
([06-roadmap.md](06-roadmap.md)).

## Deferred (post-v1 follow-ups)

- Load/envelope test: seed low-thousands of tasks, endpoints and graph stay responsive.
- Visual regression on the graph lenses.
- Accessibility checks (axe in Playwright).
