#!/usr/bin/env bash
# drop-turbo-dom-pin.sh — remove the pre-publish [patch.crates-io] pin.
#
# Pre-tag step (PUBLISHING.md "Pre-tag: drop the pre-publish patch"):
# release-guard.sh fails while the section remains. After this, both turbo-dom
# crates resolve from crates.io.
#
# Usage: scripts/agent/drop-turbo-dom-pin.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,10p' "$0"; exit 0
fi

root="$(repo_root)"
toml="$root/Cargo.toml"
tmp="$(mktemp)"

awk '
  /^\[patch\.crates-io\]/ { in_patch=1; next }
  in_patch && /^\[/ && !/^\[patch/ { in_patch=0; print; next }
  in_patch { next }
  { print }
' "$toml" > "$tmp"
awk 'BEGIN{blanks=0} /^$/{blanks++; next} {for(i=0;i<blanks;i++) print ""; blanks=0; print}' "$tmp" > "$toml"
rm -f "$tmp"

cd "$root"
cargo update -p turbo-dom-layout -p turbo-dom >/dev/null
if grep -q '^\[patch\.crates-io\]' "$toml"; then
  echo "error: [patch.crates-io] still present after drop; inspect $toml" >&2
  exit 1
fi
echo "✓ dropped turbo-dom pin; Cargo.toml + Cargo.lock refreshed from crates.io"
