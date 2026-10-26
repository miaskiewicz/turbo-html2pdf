#!/bin/sh
# Tag-only release guard (release.yml, release-crates.yml, release-py.yml).
#
# Pre-publish, the root Cargo.toml's [patch.crates-io] resolves turbo-dom-layout and
# turbo-dom from git (PUBLISHING.md). A release tag must build against the published
# registry crates, so this fails while that section exists or Cargo.lock still holds a
# git source, and when a versioned manifest disagrees with the tag.
#
# Usage: scripts/release-guard.sh TAG      (TAG = vX.Y.Z or pyvX.Y.Z)
set -eu
tag=${1:?usage: release-guard.sh vX.Y.Z|pyvX.Y.Z}
case $tag in
  pyv*) version=${tag#pyv} ;;
  v*) version=${tag#v} ;;
  *) echo "release-guard: not a release tag: $tag" >&2; exit 2 ;;
esac
cd "$(dirname "$0")/.."
fail=0
err() { echo "::error::$*"; fail=1; }

if grep -n '^\[patch' Cargo.toml; then
  err "Cargo.toml still has a [patch] section (drop it and re-lock before tagging: PUBLISHING.md)"
fi
if grep -n '^source = "git+' Cargo.lock; then
  err "Cargo.lock still resolves crates from git (re-lock from crates.io: PUBLISHING.md)"
fi

same() { [ "$2" = "$version" ] || err "$1 version '$2' != tag version '$version'"; }
same "Cargo.toml [workspace.package]" \
  "$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml)"
for pkg in crates/turbo-pdf-napi packages/react packages/template; do
  same "$pkg/package.json" "$(node -p "require('./$pkg/package.json').version")"
done
same crates/turbo-pdf-py/pyproject.toml \
  "$(sed -n 's/^version = "\(.*\)"$/\1/p' crates/turbo-pdf-py/pyproject.toml | head -n1)"
for crate in turbo-html2pdf-core turbo-html2pdf-mcp turbo-pdf-napi turbo-pdf-py turbo-pdf-wasm; do
  same "Cargo.lock $crate" \
    "$(sed -n "/^name = \"$crate\"$/{n;s/^version = \"\(.*\)\"$/\1/p;}" Cargo.lock)"
done
# The mcp crate's registry pin on core tracks the release's major.minor.
minor=${version%.*}
grep -q "path = \"../turbo-html2pdf-core\", version = \"$minor\"" crates/turbo-html2pdf-mcp/Cargo.toml ||
  err "crates/turbo-html2pdf-mcp/Cargo.toml must pin turbo-html2pdf-core version = \"$minor\""

[ "$fail" -eq 0 ] && echo "release-guard: $tag ok (no [patch], no git sources, versions $version)"
exit "$fail"
