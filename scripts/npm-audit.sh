#!/usr/bin/env bash
# npm audit gate with an explicit, documented advisory allowlist.
# Every ignored id must carry a reason and a revisit condition here and in
# osv-scanner.toml; anything not listed fails the gate (GHAS policy).
set -uo pipefail

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

# npm audit exits nonzero when it finds anything, so capture and decide here.
report=$(npm audit --audit-level=low ${prefix_args[@]+"${prefix_args[@]}"} --json 2>/dev/null) || true

ids=$(jq -r '
  .vulnerabilities // {}
  | to_entries
  | map(.value.via // [])
  | flatten
  | map(select(type == "object") | .url // empty)
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
