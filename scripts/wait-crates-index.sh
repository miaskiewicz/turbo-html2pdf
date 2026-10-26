#!/bin/sh
# Wait until each named crate, at the version Cargo.lock resolves it to, is listed in
# the crates.io sparse index (release-crates.yml runs this before packaging core:
# turbo-html2pdf-core needs turbo-dom-layout + turbo-dom 0.6.0 on crates.io).
#
# Usage: scripts/wait-crates-index.sh CRATE...
#   WAIT_SECS (default 1800) bounds the wait; polls every 30 s.
set -eu
cd "$(dirname "$0")/.."
deadline=$(( $(date +%s) + ${WAIT_SECS:-1800} ))
for crate in "$@"; do
  # Exactly one registry entry: two versions (or a git/path source) fail here.
  entries=$(awk -v n="$crate" '
    $0 == "[[package]]" { name = ""; ver = "" }
    $1 == "name" { gsub(/"/, "", $3); name = $3 }
    $1 == "version" { gsub(/"/, "", $3); ver = $3 }
    $1 == "source" && name == n { print ver " " $3 }' Cargo.lock)
  count=$(printf '%s\n' "$entries" | grep -c . || true)
  [ "$count" -eq 1 ] || { echo "::error::Cargo.lock has $count entries for $crate: $entries"; exit 1; }
  version=${entries%% *}
  source=${entries#* }
  case $source in
    '"registry+https://github.com/rust-lang/crates.io-index"') ;;
    *) echo "::error::$crate $version resolves from $source, not crates.io"; exit 1 ;;
  esac
  lower=$(printf '%s' "$crate" | tr 'A-Z' 'a-z')
  case ${#lower} in
    1) path=1/$lower ;;
    2) path=2/$lower ;;
    3) path=3/$(printf '%s' "$lower" | cut -c1)/$lower ;;
    *) path=$(printf '%s' "$lower" | cut -c1-2)/$(printf '%s' "$lower" | cut -c3-4)/$lower ;;
  esac
  until curl -fsSL "https://index.crates.io/$path" 2>/dev/null | grep -q "\"vers\":\"$version\""; do
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "::error::$crate $version is not on the crates.io index (publish turbo-dom first)"
      exit 1
    fi
    echo "waiting for $crate $version on the crates.io index..."
    sleep 30
  done
  echo "$crate $version is on crates.io"
done
