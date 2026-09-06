#!/usr/bin/env bash
# Smoke check: boot → /health → spec → UI index. Seconds-fast; gates every PR.
# Assumes `ui/dist` is built and the workspace compiles (CI builds both first).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${SMOKE_PORT:-7541}"
BASE="http://127.0.0.1:${PORT}"

fail() {
  echo "smoke: FAIL — $1" >&2
  exit 1
}

[ -f "$ROOT/ui/dist/index.html" ] || fail "ui/dist/index.html missing — build the UI first"

(cd "$ROOT/core" && cargo build -q -p shepherd-server)

"$ROOT/core/target/debug/shepherd-server" --port "$PORT" --ui-dir "$ROOT/ui/dist" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

# Boot: poll /health until the server answers, up to 5 seconds.
booted=false
for _ in $(seq 1 50); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then
    booted=true
    break
  fi
  kill -0 "$SERVER_PID" 2>/dev/null || fail "server exited during boot"
  sleep 0.1
done
[ "$booted" = true ] || fail "server did not answer /health within 5s"

health="$(curl -fsS "$BASE/health")" || fail "/health unreachable"
echo "$health" | grep -q '"status":"ok"' || fail "/health did not report ok: $health"

spec="$(curl -fsS "$BASE/api/v1/openapi.yaml")" || fail "spec unreachable"
echo "$spec" | grep -q '^openapi:' || fail "served spec is not an OpenAPI document"

index="$(curl -fsS "$BASE/")" || fail "UI index unreachable"
echo "$index" | grep -q '<div id="root">' || fail "UI index is not the app shell"

echo "smoke: OK — health, spec, and UI index served on $BASE"
