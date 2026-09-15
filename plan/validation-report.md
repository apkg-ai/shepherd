# Handbook validation report

Completed on 2026-09-14 and revised after the scaffold-reset decision. This change adds the implementation handbook under `plan/` only. No application source, existing contracts, package lockfiles or user data were changed. All 29 implementation steps remain **not started**.

## Observed checks

| Check | Result |
|---|---|
| `python3 plan/validate.py` | PASS: all local Markdown links, contract references, 29-step acyclic dependency graph, required step sections, 70-operation catalog/adapter/example parity, path parameters and Python syntax. |
| SQLite baseline initialization | PASS: fresh strict tables, integrity_check, foreign_key_check, and actual reference-project inserts. Partial block/waiver/cancellation/archive records are rejected. |
| `node plan/validate-contracts.cjs` | PASS: 108 request/response examples, portable reference fixture, rejection of null epic membership and unknown task fields. Integer/UUID/date/URI formats are checked. |
| OpenAPI Spectral with `plan/contracts/spectral.yaml` | PASS: no warnings or errors. |
| AsyncAPI Spectral with `plan/contracts/spectral-events.yaml` | PASS: no warnings or errors. |
| `openapi-to-rust 0.16.0` model generation | PASS: 94 schemas and 70 operations analyzed; generated models compile in an isolated Rust crate. |
| Generated Axum server traits/router/validation | PASS: isolated server generation and `cargo check --offline`, excluding the three deliberately handwritten streaming operations. |
| Orval 8.31.0 React Query v5 and Zod generation | PASS: generated in a temporary directory and typechecked with the repository TypeScript tool and strict settings, including erasableSyntaxOnly. |
| Reference workflow script | Syntax checked only. It was not run against the MVP because it targets the future v1 API and creates sample project data. |

The combined generation/compilation check is reproducible with:

```sh
python3 plan/check-generation.py
```

This machine required `--node-dir /Users/sheplu/.nvm/versions/node/v24.19.0/bin`; installed Node was 24.19.0, while the repository's `.nvmrc` requires **26** for application development/release. The generated outputs compiled under the available Node, but this is not a substitute for the future Node-26 application CI gate. Rust/Cargo 1.98.1 and generator 0.16.0 matched repository pins. No dependency downloads were needed for isolated Rust checks.

## Findings resolved during authoring

- Persist cancellation/archive actor, reason and timestamp in SQL, response schemas and exports; preserve command reasons in durable events.
- Reject partially populated SQL records explicitly rather than letting a NULL CHECK expression pass.
- Distinguish review claim acquisition from permission to decide while holding that claim.
- Specify server-derived session timestamps, failure-reason defaults, and phase reset when planning is disabled.
- Remove arbitrary portable-export collection caps; specify owner-only staged streaming import/export and storage-exhaustion behavior.
- Move portability before complete REST integration so no catalog endpoint depends on a later unfinished feature.
- Add step 000 to remove MVP product behavior while preserving a green build, generation, test and CI scaffold; later work now uses final module paths instead of a parallel `v1/` namespace.
- Set TanStack Query v5 explicitly in scratch Orval generation; without project context Orval defaults to v4.
- Add a stable local-daemon lint profile. The existing MVP profile requires cloud-style HTTPS/rate-limit declarations inconsistent with the selected local transport and actual behavior; the exact replacement and rationale are documented in contracts/README.md.

## What this does not claim

The v1 domain, authentication, UI, adapters, backup/recovery, new runtime dependencies and packaged releases have not been implemented or exercised. Full application CI, live multi-agent workflows, target-platform packaging and performance budgets are acceptance work in the numbered steps. Schema examples demonstrate valid shapes; the live workflow and acceptance matrix define behavioral expectations. Do not mark implementation complete based on this report.
