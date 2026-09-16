#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${HURL_PORT:-7542}"
BASE="http://127.0.0.1:${PORT}"
SERVER_PID=""

for tool in hurl curl cargo; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "hurl-e2e: missing required tool: $tool" >&2
    exit 1
  }
done

cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "hurl-e2e: building server..."
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
  kill -0 "$SERVER_PID" 2>/dev/null || { echo "hurl-e2e: FAIL — server exited during boot" >&2; exit 1; }
  sleep 0.1
done
[ "$booted" = true ] || { echo "hurl-e2e: FAIL — server did not answer /health within 5s" >&2; exit 1; }

echo "hurl-e2e: server running on $BASE (PID $SERVER_PID)"

PASS=0
FAIL=0

while IFS= read -r f; do
  name="$(basename "$f")"
  if hurl --variable "base_url=$BASE" --test "$f" 2>&1; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "hurl-e2e: FAIL — $name"
  fi
done < <(find "$ROOT/tests/hurl" -name '*.hurl' | sort)

echo ""
echo "hurl-e2e: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
