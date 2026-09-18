# shepherd

A local-first hub for long, multi-session agentic projects. It never spawns or orchestrates agents — it is a passive hub (storage + REST API + UI) that agentic tools query for their next task and report back to.

Current state: the v1 step-000 scaffold — a health-only Rust daemon (axum) serving the built React shell, with the full build, generation, test, and CI pipeline. The stable-v1 domain is rebuilt step by step from the [plan/](plan/steps/README.md) handbook (current step: [000 — scaffold reset](plan/steps/000-scaffold-reset.md)); contracts live in [plan/contracts/](plan/contracts/), and the contract actually served is [openapi/shepherd.yaml](openapi/shepherd.yaml).

## Quickstart

Toolchain pins: Rust from [rust-toolchain.toml](rust-toolchain.toml), Node from [.nvmrc](.nvmrc), and `openapi-to-rust` 0.17.0 for code generation.

```sh
cargo install openapi-to-rust --version 0.17.0 --locked
scripts/regen-generated.sh                  # Rust wire types (gitignored)
npm ci && npm ci --prefix ui
(cd ui && node --run generate:api)          # TS client + MSW mocks (gitignored)
(cd ui && node --run build)
cargo run -p shepherd-server --manifest-path core/Cargo.toml
# → http://127.0.0.1:7437 (override with --port / SHEPHERD_PORT)
```

The scaffold has no database: the server reads and writes no user data.

## Quality gates

Every PR runs the full pipeline (`.github/workflows/`):

| Gate | What it checks |
|---|---|
| Core | `cargo fmt`, Clippy `-D warnings`, tests with llvm-cov |
| Contract | Live responses validated against the served OpenAPI spec |
| UI | oxlint + oxfmt, `tsc -b`, Vitest with coverage, Vite build |
| Spec lint | Spectral (OAS + OWASP + IBM + APIs-You-Won't-Hate) |
| E2E / smoke | Hurl API checks, Playwright + axe WCAG-AA audit, boot smoke |
| Coverage | Line-coverage thresholds per suite (`scripts/coverage-report.ts`) |
| Security | osv-scanner, npm/cargo audit, Semgrep |
