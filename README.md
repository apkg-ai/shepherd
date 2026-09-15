# shepherd

Shepherd is a local-first hub for long, multi-session agentic projects, being
rebuilt toward a stable v1. It never spawns or orchestrates agents — it is a
passive central hub (storage + REST API + visualization) that agentic tools
query for their next task and report back to.

**Current state: the v1 step-000 green scaffold.** The repository contains a
health-only Rust daemon (axum) that serves the built React shell, plus the
full build, generation, test, and CI pipeline. The MVP product implementation
was removed; the stable-v1 domain is rebuilt step by step on this scaffold.

## The plan

The `plan/` handbook is the authority for all v1 work:

- [Execution rules and step index](plan/steps/README.md) — current step:
  [000 — scaffold reset](plan/steps/000-scaffold-reset.md); next eligible: 001.
- [Product scope](plan/00-product-scope.md), [domain model](plan/03-domain-model.md),
  [backend](plan/05-backend.md), [frontend](plan/06-frontend.md),
  [test strategy](plan/14-test-strategy.md).
- The full v1 contracts live in [plan/contracts/](plan/contracts/); the
  application currently serves the health-only scaffold contract
  ([openapi/shepherd.yaml](openapi/shepherd.yaml)).

## Quickstart

Toolchain pins: Rust from [rust-toolchain.toml](rust-toolchain.toml), Node
from [.nvmrc](.nvmrc), and `openapi-to-rust` 0.16.0 for code generation.

```sh
cargo install openapi-to-rust --version 0.16.0 --locked
scripts/regen-generated.sh                  # Rust wire types (gitignored)
npm ci && npm ci --prefix ui
npm run generate:api --prefix ui            # TS client + MSW mocks (gitignored)
npm run build --prefix ui
cargo run -p shepherd-server --manifest-path core/Cargo.toml
# → http://127.0.0.1:7437 (override with --port / SHEPHERD_PORT)
```

The scaffold has no database: the server reads and writes no user data.

## Quality gates

Every PR runs the full pipeline (`.github/workflows/`):

| Gate | What it checks |
|---|---|
| Core lint / unit / integration | `cargo fmt`, Clippy `-D warnings`, workspace tests with llvm-cov |
| Contract | Live responses validated against the served OpenAPI spec |
| UI lint / typecheck / unit / build | oxlint + oxfmt, `tsc -b`, Vitest with coverage, Vite build |
| Spec lint | Spectral (OAS + OWASP + IBM + APIs-You-Won't-Hate) on the contract |
| Hurl / Playwright / smoke | Real-server API checks, browser + axe WCAG-AA audit, boot smoke |
| Coverage report | Line-coverage thresholds gated per suite (`scripts/coverage-report.mjs`) |
| Dependency scan / SAST | osv-scanner, npm/cargo audit, Semgrep |
