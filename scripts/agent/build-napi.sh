#!/usr/bin/env bash
# build-napi.sh — reproducible N-API build for turbo-html2pdf.
#
# Runs `npm ci` + `napi build --platform --release` inside crates/turbo-pdf-napi.
# Keeps the heavy cargo target directory out of the worktree (shared machine
# hygiene) and points rusty_v8 at the cached archive so cold builds do not
# redownload or rebuild V8.
#
# Usage:
#   scripts/agent/build-napi.sh [--debug] [--features F1,F2]
#
# Env overrides:
#   SCRATCH_ROOT              base for CARGO_TARGET_DIR (default /tmp/turbo-html2pdf-agent-<user>)
#   RUSTY_V8_ARCHIVE_DIR      directory with the cached rusty_v8 .tar.gz
#   RUSTY_V8_ARCHIVE          explicit path to the archive (overrides discovery)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=_lib.sh
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,20p' "$0"; exit 0
fi

profile=release
features=""
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) profile=debug; shift ;;
    --features) features="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

root="$(repo_root)"
scratch="$(ensure_scratch)"
export CARGO_TARGET_DIR="$scratch/target-napi"

v8dir="$(rusty_v8_archive_dir)"
if [ -z "${RUSTY_V8_ARCHIVE:-}" ] && [ -d "$v8dir" ]; then
  archive="$(find "$v8dir" -maxdepth 1 \( -name 'librusty_v8_release_*.a.gz' -o -name 'librusty_v8_release_*.tar.gz' \) 2>/dev/null | head -n1)"
  [ -n "$archive" ] && export RUSTY_V8_ARCHIVE="$archive"
fi
[ -n "${RUSTY_V8_ARCHIVE:-}" ] && echo "RUSTY_V8_ARCHIVE=$RUSTY_V8_ARCHIVE"

cd "$root"
echo "• npm ci (root)"
npm ci --no-audit --no-fund

cd "$root/crates/turbo-pdf-napi"
echo "• napi build ($profile${features:+ , --features $features})"
args=(--platform --dts index.generated.d.ts --js index.generated.js)
[ "$profile" = release ] && args+=(--release)
[ -n "$features" ] && args+=(--features "$features")
npx napi build "${args[@]}"

echo "✓ napi build complete (target: $CARGO_TARGET_DIR)"
