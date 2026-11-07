#!/usr/bin/env bash
# release-prep.sh X.Y.Z — stamp every version string the release runbook lists.
#
# Updates (in lockstep):
#   Cargo.toml                                   workspace.package.version
#   crates/turbo-pdf-napi/package.json           "version"
#   packages/react/package.json                  "version"
#   packages/template/package.json               "version"
#   crates/turbo-pdf-py/pyproject.toml           version = "..."
#   crates/turbo-pdf-napi/README.md              "## Status" vX.Y.Z line
#   benches/competitive/src/adapters/turbopdf.ts version: "..."
#   crates/turbo-html2pdf-mcp/Cargo.toml         core dep "^X.Y"  (minor bumps only)
#
# Then runs both release guards (scripts/release-guard.sh v* and pyv*).
# Does NOT commit, tag, or push. Review `git diff` first.
#
# Usage: scripts/agent/release-prep.sh X.Y.Z
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,20p' "$0"; exit 0
fi

ver="${1:?usage: release-prep.sh X.Y.Z}"
case "$ver" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "error: '$ver' is not X.Y.Z" >&2; exit 2 ;;
esac

root="$(repo_root)"
cd "$root"

replace() {  # replace FILE PATTERN REPLACEMENT
  local f="$1" p="$2" r="$3"
  [ -f "$f" ] || { echo "skip (missing): $f"; return; }
  if ! grep -qE "$p" "$f"; then
    echo "warn: pattern not found in $f: $p" >&2; return
  fi
  sed -i.bak -E "s|$p|$r|" "$f" && rm "$f.bak"
  echo "updated: $f"
}

replace Cargo.toml                                       '^version = "[0-9.]+"'      "version = \"$ver\""
replace crates/turbo-pdf-napi/package.json               '"version": "[0-9.]+"'      "\"version\": \"$ver\""
replace packages/react/package.json                      '"version": "[0-9.]+"'      "\"version\": \"$ver\""
replace packages/template/package.json                   '"version": "[0-9.]+"'      "\"version\": \"$ver\""
replace crates/turbo-pdf-py/pyproject.toml               '^version = "[0-9.]+"'      "version = \"$ver\""
replace crates/turbo-pdf-napi/README.md                  'v[0-9]+\.[0-9]+\.[0-9]+'   "v$ver"
replace benches/competitive/src/adapters/turbopdf.ts     'version: "[0-9.]+"'        "version: \"$ver\""

# MCP crate: core dep carries an explicit "^X.Y" alongside its path (crates.io
# requires a version). Bump only on a minor change (CLAUDE.md).
major="${ver%%.*}"; rest="${ver#*.}"; minor="${rest%%.*}"
want="$major.$minor"
mcp_toml="crates/turbo-html2pdf-mcp/Cargo.toml"
if [ -f "$mcp_toml" ]; then
  cur=$(grep -Eo 'turbo-html2pdf-core = \{ path = "\.\./turbo-html2pdf-core", version = "[0-9]+\.[0-9]+"' "$mcp_toml" \
        | grep -Eo '"[0-9]+\.[0-9]+"' | tr -d '"' || true)
  if [ -n "$cur" ] && [ "$cur" != "$want" ]; then
    sed -i.bak -E "s|(version = )\"$cur\"|\1\"$want\"|" "$mcp_toml" && rm "$mcp_toml.bak"
    echo "updated: $mcp_toml  (core pin $cur -> $want)"
  fi
fi

echo "regenerating Cargo.lock"
cargo update -w >/dev/null

echo
echo "== release guards =="
sh scripts/release-guard.sh "v$ver"
sh scripts/release-guard.sh "pyv$ver"
echo
echo "✓ release-prep $ver complete. Review 'git diff', run full gates, then tag the merge commit."
