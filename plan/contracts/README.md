# Target contracts

These are v1 design artifacts; the running MVP does not implement them. The `.yaml` files use JSON syntax, which is valid YAML 1.2, to make local parsing and schema checks deterministic without adding dependencies. The OpenAPI file owns wire fields; the domain/workflow documents own their meaning. `operations.json` is a mechanically derived catalog used by step, CLI, and MCP mappings.

Absence: response collections are always present. A singular optional relationship is represented by a bounded array of zero or one IDs/revisions; this avoids nullable-reference inconsistencies in the existing generators. Do not turn these into arbitrary multiple relationships. Optional request fields omitted on creation use documented defaults; patch omission preserves a value. Explicit null is invalid. Empty patch is 422.

Run `python3 plan/validate.py` and `PATH=/Users/sheplu/.nvm/versions/node/v24.19.0/bin:$PATH node plan/validate-contracts.cjs` on this machine. On other installations use Node from `.nvmrc`. Validation evidence and generator limitations are recorded in ../validation-report.md.

## Stable-v1 lint policy

Run `node_modules/.bin/spectral lint plan/contracts/openapi.yaml --ruleset plan/contracts/spectral.yaml --fail-severity=warn` and the analogous events check with spectral-events.yaml. At REST integration, promote these profiles and update package lint scripts to their installed locations. Retain standard OpenAPI/AsyncAPI schema, operation and example checks.

The MVP profile is not carried over unchanged: it requires HTTPS for a plain-loopback daemon, fake rate headers/429 responses without a limiter, IBM-specific enum naming incompatible with event names/version enums, model-only responses incompatible with SSE, and maximum array sizes incompatible with streaming project portability. V1 uses truthful transport, explicit per-record limits and authenticated streaming import. No security claim is satisfied by decorative headers. Additional semantic/auth/range checks live in plan/validate.py and the core/HTTP acceptance tests. Lint policy changes are an explicit step-015 change, not silent suppressions in application code.
