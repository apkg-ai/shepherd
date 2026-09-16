#!/usr/bin/env bash
# Regenerates src/generated (gitignored) from openapi/shepherd.yaml — run
# before cargo build/test. cargo fmt fixes the unformatted generator output.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

command -v openapi-to-rust >/dev/null 2>&1 || {
  echo "regen: openapi-to-rust not found. Install the pinned version:" >&2
  echo "regen:   cargo install openapi-to-rust --version 0.17.0 --locked" >&2
  exit 1
}

(cd "$ROOT/core/shepherd-server" && openapi-to-rust generate)
(cd "$ROOT/core" && cargo fmt)

echo "regen: Rust wire types regenerated"
