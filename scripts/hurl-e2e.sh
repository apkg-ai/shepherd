#!/usr/bin/env bash
# E2E API tests using hurl. Boots the server on a fresh in-memory DB,
# runs all .hurl files, then tears down.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${HURL_PORT:-7542}"
BASE="http://127.0.0.1:${PORT}"
DB_DIR="$(mktemp -d)"
DB_PATH="${DB_DIR}/hurl-test.db"

cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  rm -rf "$DB_DIR"
}
trap cleanup EXIT

# Build the server.
echo "hurl-e2e: building server..."
(cd "$ROOT/core" && cargo build -q -p shepherd-server)

# Boot with a fresh temporary DB.
"$ROOT/core/target/debug/shepherd-server" \
  --port "$PORT" \
  --ui-dir "$ROOT/ui/dist" \
  --db "$DB_PATH" &
SERVER_PID=$!

# Poll until healthy.
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

# Run all hurl files in order.
HURL_FILES="$(find "$ROOT/tests/hurl" -name '*.hurl' | sort)"
PASS=0
FAIL=0

for f in $HURL_FILES; do
  name="$(basename "$f")"
  if hurl --variable "base_url=$BASE" --test "$f" 2>&1; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "hurl-e2e: FAIL — $name"
  fi
done

echo ""
echo "hurl-e2e: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
