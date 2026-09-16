#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${PLAYWRIGHT_PORT:-7543}"
BASE="http://127.0.0.1:${PORT}"
SERVER_PID=""

for tool in curl cargo npx; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "playwright-e2e: missing required tool: $tool" >&2
    exit 1
  }
done

[ -f "$ROOT/ui/dist/index.html" ] || {
  echo "playwright-e2e: ui/dist/index.html missing — build the UI first (npm run build)" >&2
  exit 1
}

cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "playwright-e2e: building server..."
(cd "$ROOT/core" && cargo build -q -p shepherd-server)

"$ROOT/core/target/debug/shepherd-server" \
  --port "$PORT" \
  --ui-dir "$ROOT/ui/dist" &
SERVER_PID=$!

booted=false
for _ in $(seq 1 50); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then
    booted=true
    break
  fi
  kill -0 "$SERVER_PID" 2>/dev/null || { echo "playwright-e2e: FAIL — server exited during boot" >&2; exit 1; }
  sleep 0.1
done
[ "$booted" = true ] || { echo "playwright-e2e: FAIL — server did not answer /health within 5s" >&2; exit 1; }

echo "playwright-e2e: server running on $BASE (PID $SERVER_PID)"

(cd "$ROOT/ui" && PLAYWRIGHT_BASE_URL="$BASE" npx playwright test "$@")
