#!/usr/bin/env bash
# record-chrome-control.sh — run a Chrome-156 control probe and save its fixture.
#
# Wraps the headless-shell oracle with the flags every sibling agent uses:
#   --use-mock-keychain --password-store=basic --headless=new
# Probes live under scripts/*-chrome-control.mjs. Localhost / file:// only.
#
# Usage:
#   scripts/agent/record-chrome-control.sh PROBE OUT
#
# Example:
#   scripts/agent/record-chrome-control.sh \
#     scripts/pagination-chrome-control.mjs \
#     crates/turbo-html2pdf-core/tests/fixtures/pagination_chrome156.json
#
# Env:
#   CHROME   override the headless-shell binary path
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,20p' "$0"; exit 0
fi

if [ $# -lt 2 ]; then
  echo "usage: $(basename "$0") PROBE OUT" >&2; exit 2
fi
probe="$1"
out="$2"

root="$(repo_root)"
cd "$root"
chrome="${CHROME:-$(chrome_headless_shell)}"
if [ ! -x "$chrome" ]; then
  echo "error: Chrome 156 headless-shell not found at: $chrome" >&2
  echo "       set CHROME=... (sibling agents share one cache)" >&2
  exit 2
fi
if [ ! -f "$probe" ]; then
  echo "error: probe not found: $probe" >&2; exit 2
fi
mkdir -p "$(dirname "$out")"

export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$chrome"
export TURBO_CHROME_FLAGS="--use-mock-keychain --password-store=basic"

echo "chrome: $chrome"
echo "probe:  $probe"
echo "out:    $out"
exec node "$probe" --chrome "$chrome" --out "$out"
