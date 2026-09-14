# 016 — Browser session and hierarchy navigation

Status: not started. Requirements: UI-01 AUTH-01.

## Objective and prerequisites

Deliver browser session and hierarchy navigation. Required completed steps: [015](015-api-cutover.md)

Read [execution rules](README.md) first, then:

- [06-frontend.md](../06-frontend.md)
- [07-rest-contract.md](../07-rest-contract.md)
- [08-events-and-history.md](../08-events-and-history.md)

Starting state: prerequisite step completion checks pass and their handoff records describe the actual code. The schemas and transition tables in plan/ are authoritative, not old MVP docs. Any temporary interfaces below must be private to core and backed by tests; no unimplemented success endpoint may be exposed.

## Files and boundaries

- `ui/src/router.tsx`
- `ui/src/api/client.ts`
- `ui/src/screens/AppLayout.tsx`
- `ui/src/screens/identity/LoginScreen.tsx`
- `ui/src/screens/goals/{GoalFormScreen,GoalDetailScreen}.tsx`
- `ui/src/screens/projects/ProjectOverviewScreen.tsx`
- `ui/src/test/{fixtures,msw}.ts`

Tests: `Adjacent component tests / resource HTTP tests named for the changed behavior; retain existing target names used by CI.`. Braces denote concrete sibling filenames, not optional modules. Update related module declarations/imports and only the documented dependency manifests. Never hand-edit generated files. Follow the final backend/frontend module map and retain existing primitives.

## Ordered implementation

1. Read the existing related implementation and targeted tests. Record which functions/queries currently enforce the invariant and which need replacing.
2. Extend central fetch with cookies/CSRF/revision/idempotency. Add login/session boundary and new project/goal/epic routes, breadcrumbs and hierarchy list screens behind a local development v1 route switch until complete. Regenerate Orval models/hooks/Zod from promoted spec and update fixtures/MSW. Preserve old visual primitives; remove obsolete generated hook imports at the same cutover.
3. Implement the negative scenarios below using public domain commands or live HTTP at the appropriate boundary. Include actor, resource revision and expected state in fixtures.
4. Run the checks, repair regressions caused by this change, and update the handoff record with exact results.

## Acceptance tests and expected results

Deep links resolve; login expires cleanly; project with two goals navigates independently. Empty hierarchy has correct CTA. Fields never submit before defaults load. Existing shell/a11y tests migrate to v1 routes and pass.

For each sentence above create a named regression test with setup → action → expected status/error → persisted state checks. Mutation failures must leave resource revision/history unchanged except an independently committed prior command. Use file-backed SQLite and independent pools for concurrency, controllable clock for TTL; do not test only a mocked helper that mirrors implementation. For UI, cover loading/empty/error plus keyboard interaction for new controls, using MSW for component tests and real server for critical E2E.

## Excluded work

Do not implement subsequent steps or change confirmed product decisions. No external-agent spawning, recursive hierarchy, cross-goal dependency, server-wide auth bypass or MVP data converter. Only add dependencies pinned in [dependency choices](../dependencies.md). Never reset or delete the user's existing database to make tests pass.

## Verification

```sh
# Repository root, with Node 26 active.
npm run lint --prefix ui
npm run fmt:check --prefix ui
npm run typecheck --prefix ui
npm test --prefix ui
npm run build --prefix ui
scripts/playwright-e2e.sh
```

Run Rust commands from core/, not the repository root. Before step 015, generated MVP sources remain needed for full workspace checks; run scripts/regen-generated.sh if missing, knowing it formats Rust. At step 015 and later generate from promoted v1 contract. For UI generation run npm run generate:api --prefix ui before typecheck when contract changed. Hurl/Playwright need a fresh isolated database and their installed tools; use existing scripts rather than a personal running daemon.

Expected: zero exit status, all named acceptance cases pass, no changes outside this step's scope. These are future implementation checks, not claims that tests ran during document creation.

## Completion checklist

- [ ] Referenced requirements and every acceptance sentence implemented.
- [ ] Schemas, permissions, transitions and clients remain consistent.
- [ ] Success and rejection behavior verified at the public boundary.
- [ ] Required commands pass; material environmental limitation recorded accurately.
- [ ] Temporary code and next-step dependencies documented.
- [ ] Handoff below completed; next eligible step linked.

## Implementation handoff record

Fill when implementing, not during planning: commit/branch; files changed; test commands and results; any reproduced limitation; temporary interfaces; next eligible step IDs. If an acceptance criterion cannot pass, leave this step incomplete and explain the concrete blocker. Do not mark complete for code that merely compiles.
