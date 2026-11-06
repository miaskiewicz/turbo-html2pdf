#!/usr/bin/env bash
# parity-compare.sh — compare html2pdf output to Chrome using turbo-dom's parity tool.
#
# turbo-dom's parity harness (tools/parity/parity.py) renders inputs with both
# Chrome 156 and the engine under test and diffs the resulting artifacts. This
# wrapper points it at turbo-html2pdf's expected files (default: the core crate
# fixtures). The sibling turbo-dom checkout must already have run
# `tools/parity/fetch-inputs.sh`.
#
# Usage:
#   scripts/agent/parity-compare.sh [--dom PATH] [--inputs GLOB] [-- parity.py args...]
#
# Env:
#   TURBO_DOM_DIR   path to the turbo-dom checkout (default: ../turbo-dom)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,15p' "$0"; exit 0
fi

dom="${TURBO_DOM_DIR:-$(cd "$(repo_root)/.." && pwd)/turbo-dom}"
inputs=""
extra=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dom) dom="$2"; shift 2 ;;
    --inputs) inputs="$2"; shift 2 ;;
    --) shift; extra=("$@"); break ;;
    *) extra+=("$1"); shift ;;
  esac
done

parity="$dom/tools/parity/parity.py"
if [ ! -f "$parity" ]; then
  echo "error: parity tool not found: $parity" >&2
  echo "       set TURBO_DOM_DIR to a turbo-dom checkout" >&2
  exit 2
fi

root="$(repo_root)"
expected_default="$root/crates/turbo-html2pdf-core/tests/fixtures"
inputs="${inputs:-$expected_default}"

echo "parity: dom=$dom  inputs=$inputs"
exec python3 "$parity" --engine turbo-html2pdf --engine-root "$root" --inputs "$inputs" "${extra[@]}"
