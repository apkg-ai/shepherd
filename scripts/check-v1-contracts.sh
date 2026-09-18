#!/usr/bin/env bash
# v1 contract baseline gate (plan/steps/001-contract-baseline.md).
# Pass --node-dir <dir> if Node from .nvmrc is not on PATH.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NODE_DIR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --node-dir)
      NODE_DIR="$2"
      shift 2
      ;;
    *)
      echo "usage: $0 [--node-dir <node-bin-directory>]" >&2
      exit 2
      ;;
  esac
done
if [ -n "$NODE_DIR" ]; then
  PATH="$NODE_DIR:$PATH"
fi

fail() {
  echo "check-v1-contracts: FAIL — $1" >&2
  exit 1
}

for tool in node python3 cargo openapi-to-rust; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing required tool: $tool (see --node-dir for Node)"
done
[ -d "$ROOT/node_modules" ] || fail "root node_modules missing — run: npm ci"
[ -d "$ROOT/ui/node_modules" ] || fail "ui/node_modules missing — run: npm ci --prefix ui"

# The generator pin's single source is the CI composite action.
GENERATOR_VERSION="$(grep -m1 'default:' "$ROOT/.github/actions/generate-wire-types/action.yml" | cut -d'"' -f2)"
[ -n "$GENERATOR_VERSION" ] || fail "could not parse the openapi-to-rust pin from .github/actions/generate-wire-types/action.yml"
openapi-to-rust --version | grep -q "openapi-to-rust ${GENERATOR_VERSION}$" ||
  fail "openapi-to-rust $(openapi-to-rust --version | cut -d' ' -f2) does not match the pin — run: cargo install openapi-to-rust --version ${GENERATOR_VERSION} --locked"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/shepherd-v1-contracts.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

step() {
  echo ""
  echo "── $1 ──"
}

# No pipelines: a failing assertion must never read as success.
expect_failure() {
  local name="$1" needle="$2" out status=0
  shift 2
  out="$("$@" 2>&1)" || status=$?
  if [ "$status" -eq 0 ]; then
    echo "$out"
    fail "[$name] expected non-zero exit"
  fi
  if [[ "$out" != *"$needle"* ]]; then
    echo "$out"
    fail "[$name] output does not mention '$needle'"
  fi
  echo "ok [$name]: rejected as expected (exit $status)"
}

# Warm caches stay offline; cold caches fetch once from the network.
cargo_fetch_offline_first() {
  cargo fetch --offline --manifest-path "$1" >/dev/null 2>&1 || cargo fetch --manifest-path "$1"
}

# $1 = crate dir with src/generated, $2 = crate name.
temp_crate_manifest() {
  {
    printf '[package]\nname = "%s"\nversion = "0.0.0"\nedition = "2024"\n' "$2"
    cat "$1/src/generated/REQUIRED_DEPS.toml"
  } >"$1/Cargo.toml"
  printf 'pub mod generated;\n' >"$1/src/lib.rs"
}

step "validate-plan: handbook references, step DAG, schema refs"
python3 "$ROOT/plan/validate.py"

step "validate-examples: catalog examples against schemas"
node "$ROOT/plan/validate-contracts.cjs"

step "lint-openapi: spectral stable-v1 profile"
"$ROOT/node_modules/.bin/spectral" lint "$ROOT/plan/contracts/openapi.yaml" \
  --ruleset "$ROOT/plan/contracts/spectral.yaml" --fail-severity=warn

step "lint-asyncapi: spectral events profile"
"$ROOT/node_modules/.bin/spectral" lint "$ROOT/plan/contracts/asyncapi.yaml" \
  --ruleset "$ROOT/plan/contracts/spectral-events.yaml" --fail-severity=warn

step "contract-smoke: every schema compiles, every inline example validates"
node "$ROOT/scripts/check-contract-smoke.ts"

step "operation-freeze: spec operation IDs match the committed freeze list"
node "$ROOT/scripts/check-operation-freeze.ts"

step "negative-broken-ref: unresolved schema reference is detected"
cp -R "$ROOT/plan" "$TMP/broken-ref"
node "$ROOT/scripts/contract-fixtures.ts" mutate broken-ref "$TMP/broken-ref"
expect_failure negative-broken-ref '__MissingSchema' python3 "$TMP/broken-ref/validate.py"

step "negative-corrupt-example: invalid operation example is detected"
cp -R "$ROOT/plan" "$TMP/corrupt-example"
node "$ROOT/scripts/contract-fixtures.ts" mutate corrupt-example "$TMP/corrupt-example"
# The temp copy has no node_modules ancestor; resolve ajv from the repo root.
expect_failure negative-corrupt-example 'Health' \
  env NODE_PATH="$ROOT/node_modules" node "$TMP/corrupt-example/validate-contracts.cjs"

step "negative-renamed-operation: operation-ID drift is detected"
cp -R "$ROOT/plan" "$TMP/renamed-op"
node "$ROOT/scripts/contract-fixtures.ts" mutate renamed-operation "$TMP/renamed-op"
expect_failure negative-renamed-operation 'getHealthRenamed' \
  node "$ROOT/scripts/check-operation-freeze.ts" --spec "$TMP/renamed-op/contracts/openapi.yaml"

step "client-module: Rust-client wire models generate and compile from the same source"
CLIENT="$TMP/client-crate"
openapi-to-rust generate "$ROOT/plan/contracts/openapi.yaml" \
  --output-dir "$CLIENT/src/generated" --module-name shepherd --json
[ -f "$CLIENT/src/generated/client.rs" ] || fail "client.rs not generated"
[ -f "$CLIENT/src/generated/types.rs" ] || fail "types.rs not generated"
temp_crate_manifest "$CLIENT" shepherd-v1-client-check
cargo_fetch_offline_first "$CLIENT/Cargo.toml"
cargo check --offline --quiet --manifest-path "$CLIENT/Cargo.toml"
echo "ok [client-module]: generated client + wire models compile"

step "warm-generation-deps: fetch models/server REQUIRED_DEPS for check-generation --offline"
WARM="$TMP/warm-server"
node "$ROOT/scripts/contract-fixtures.ts" server-config "$ROOT" "$TMP/warm-server.toml" "$WARM/src/generated"
openapi-to-rust generate --config "$TMP/warm-server.toml" --json
temp_crate_manifest "$WARM" shepherd-v1-warm-server
cargo_fetch_offline_first "$WARM/Cargo.toml"
WARM_MODELS="$TMP/warm-models"
openapi-to-rust generate "$ROOT/plan/contracts/openapi.yaml" --types-only \
  --output-dir "$WARM_MODELS/src/generated" --json
temp_crate_manifest "$WARM_MODELS" shepherd-v1-warm-models
cargo_fetch_offline_first "$WARM_MODELS/Cargo.toml"

step "generation: temporary Rust models/server and Orval React Query/Zod output"
if [ -n "$NODE_DIR" ]; then
  python3 "$ROOT/plan/check-generation.py" --node-dir "$NODE_DIR"
else
  python3 "$ROOT/plan/check-generation.py"
fi

echo ""
echo "check-v1-contracts: PASS — all v1 contract baseline checks green"
