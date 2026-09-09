#!/usr/bin/env bash
# Regenerate core/shepherd-server/src/generated from openapi/shepherd.yaml.
#
# The generator's raw output is not rustfmt-formatted, so this script runs
# `cargo fmt` afterwards — the committed generated code is exactly
# regen + fmt. CI (core-generated-drift job) runs the same steps and fails
# on any diff, so spec edits cannot silently leave the wire types stale.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

command -v openapi-to-rust >/dev/null 2>&1 || {
  echo "regen: openapi-to-rust not found. Install the pinned version:" >&2
  echo "regen:   cargo install openapi-to-rust --version 0.16.0 --locked" >&2
  exit 1
}

(cd "$ROOT/core/shepherd-server" && openapi-to-rust generate)
(cd "$ROOT/core" && cargo fmt)

if git -C "$ROOT" diff --quiet -- core/shepherd-server/src/generated; then
  echo "regen: generated code is up to date"
else
  echo "regen: generated code changed — commit the result"
fi
