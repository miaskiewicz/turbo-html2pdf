#!/usr/bin/env bash
# pin-turbo-dom.sh REV — pre-publish git pin of turbo-dom{,-layout} in Cargo.toml.
#
# Same shape as the 0.7.1 release: writes a [patch.crates-io] section at the
# bottom of the workspace Cargo.toml. Drop it with drop-turbo-dom-pin.sh before
# tagging a release.
#
# Usage: scripts/agent/pin-turbo-dom.sh <rev-or-sha>
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,10p' "$0"; exit 0
fi

rev="${1:?usage: pin-turbo-dom.sh <rev-or-sha>}"
root="$(repo_root)"
toml="$root/Cargo.toml"

tmp="$(mktemp)"
awk -v rev="$rev" '
  BEGIN { in_patch=0; printed=0 }
  /^\[patch\.crates-io\]/ { in_patch=1; print; next }
  in_patch && /^\[/ && !/^\[patch/ { in_patch=0 }
  in_patch && /^turbo-dom-layout[[:space:]]*=/ { print "turbo-dom-layout = { git = \"https://github.com/miaskiewicz/turbo-dom\", rev = \"" rev "\" }"; printed=1; next }
  in_patch && /^turbo-dom[[:space:]]*=/        { print "turbo-dom = { git = \"https://github.com/miaskiewicz/turbo-dom\", rev = \"" rev "\" }"; printed=2; next }
  { print }
  END {
    if (printed == 0) {
      print ""
      print "[patch.crates-io]"
      print "turbo-dom-layout = { git = \"https://github.com/miaskiewicz/turbo-dom\", rev = \"" rev "\" }"
      print "turbo-dom = { git = \"https://github.com/miaskiewicz/turbo-dom\", rev = \"" rev "\" }"
    }
  }
' "$toml" > "$tmp"
mv "$tmp" "$toml"

cd "$root"
cargo update -p turbo-dom-layout -p turbo-dom >/dev/null
echo "✓ pinned turbo-dom{,-layout} to rev $rev (Cargo.toml + Cargo.lock)"
echo "  drop before tagging: scripts/agent/drop-turbo-dom-pin.sh"
