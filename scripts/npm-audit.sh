#!/usr/bin/env bash
# npm audit gate with an explicit, documented advisory allowlist.
# Every ignored id must carry a reason and a revisit condition here and in
# osv-scanner.toml; anything not listed fails the gate (GHAS policy).
set -uo pipefail

command -v jq >/dev/null 2>&1 || {
  echo "npm-audit: jq is required" >&2
  exit 1
}

ignored=(
  # GHSA-vfj7-8cjw-p6xm — braces stack-exhaustion DoS via deeply nested glob
  # patterns. No fix released (braces 3.0.3 is the latest published version);
  # dev-only path through spectral's globbing of our own static files.
  # Revisit: remove when braces ships a patched release.
  GHSA-vfj7-8cjw-p6xm
)

is_ignored() {
  local advisory="$1"
  local entry
  for entry in "${ignored[@]}"; do
    [[ "$advisory" == *"$entry"* ]] && return 0
  done
  return 1
}

prefix_args=()
if [[ $# -gt 0 ]]; then
  prefix_args=(--prefix "$1")
fi

# npm audit exits nonzero when it finds anything; that exit code is only
# trustworthy when it produced a parseable report, so the gate fails closed
# on operational errors (registry down, bad --prefix) instead of passing
# with nothing audited.
npm_status=0
report=$(npm audit --audit-level=low ${prefix_args[@]+"${prefix_args[@]}"} --json) || npm_status=$?
if ! jq -e 'type == "object" and (has("error") | not)' <<<"$report" >/dev/null 2>&1; then
  echo "npm-audit: npm audit failed (exit $npm_status) without a usable report" >&2
  printf '%s\n' "$report" >&2
  exit 1
fi

# Advisories without a url fall back to the package name; an object with
# neither still fails the gate rather than slipping through unreviewed.
ids=$(jq -r '
  .vulnerabilities // {}
  | to_entries
  | map(.value.via // [])
  | flatten
  | map(select(type == "object") | (.url // .name // "advisory-without-identifier"))
  | unique
  | .[]
' <<<"$report")

status=0
for advisory in $ids; do
  if is_ignored "$advisory"; then
    echo "ignored advisory: $advisory"
  else
    echo "unhandled advisory: $advisory" >&2
    status=1
  fi
done

if (( status != 0 )); then
  echo "npm audit found advisories outside the allowlist" >&2
fi
exit "$status"
