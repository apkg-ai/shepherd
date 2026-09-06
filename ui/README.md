# shepherd UI

Vite + React + TypeScript shell. `npm run dev` proxies `/api` to the shepherd
daemon on `127.0.0.1:7437`; `npm run build` emits `dist/`, served by
`shepherd-server`.

Scripts: `dev`, `build`, `lint` (oxlint), `fmt` / `fmt:check` (oxfmt),
`typecheck` (tsc), `test` (vitest).
