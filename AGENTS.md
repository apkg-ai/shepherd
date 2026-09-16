# AGENTS.md

Working conventions for AI coding agents in this repository. These encode reviewed practice from the v1 step-000 sessions — follow them by default.

## Verify before committing

- Prove every change by running the relevant gates (see the README's quality-gates table); never claim "tested" without an executed command in the transcript.
- CI-only changes (workflow edits, pinned tool bumps): replicate the exact CI command locally when possible — semgrep, shellcheck, sha256 checks, contract-test filters.
- Conversions and rewrites: prove equivalence by diffing old vs new output on the same input.
- Verify claims against the actual tree before acting: fetch rule/config sources, read the generated code, run the command. Do not assert from memory.

## Comments

- One short line, only where the code cannot speak for itself: security rationale, lint-suppression reasons, non-obvious ordering/quirk traps.
- If the code is understandable in ~10 seconds, no comment.
- When a comment adds nothing, remove it entirely — no partial trims that leave narration behind. No history notes ("was removed in step 000"), no restating what a name says.

## Documentation

- READMEs carry useful information only: quickstart, gates, contribution paths. No narration.
- Markdown: one paragraph per line — no 80-character wrapping.

## Dependencies and pins

- Keep dependencies at their latest versions (`npm outdated`, `cargo update`, crates.io/GitHub releases for tools).
- CI tools are pinned by immutable hash: actions by commit SHA, binaries by sha256, container images by digest. When bumping, download the artifact and compute the hash yourself — never copy from release notes.
- Deliberate pins, do not "fix" them: `jsonschema = "0.49"` is the generator's contract (`REQUIRED_DEPS.toml`); the npm `lodash` override is security-required (removing it resolves vulnerable lodash 4.17.23).

## CI

- Deduplicate repeated workflow steps into composite actions (`.github/actions/*`).
- Use `node --run`, never `npm run` (Node 26 from `.nvmrc`; `node --run` skips npm pre/post hooks — we use none).
- Shell scripts under `scripts/` run in CI: keep them shellcheck-clean (pinned 0.11.0 gate).
- `plan/` and `*.test.tsx` are excluded from the semgrep scan by design; product code stays fully scanned.

## UI

- All user-visible strings — including aria-labels — go through `t()`. Keys are three-segment lowercase `module.feature.name`; the semgrep `i18next-key-format` rule enforces this shape.
- UI text is English via i18next (`src/locales/`); a new language is a JSON file plus one registration line in `src/i18n.ts`.
- TypeScript everywhere, erasable syntax only (`erasableSyntaxOnly`); Node runs `.ts` directly — no `.mjs` scripts.

## Process

- Commit after each verified task; imperative commit messages.
- Never push without being asked.
