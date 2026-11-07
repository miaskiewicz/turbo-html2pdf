#!/usr/bin/env bash
# verapdf-corpus.sh — validate every rendered PDF/UA fixture with veraPDF ua1.
#
# Walks a directory of `.pdf` files (default: scratchpad/ua-corpus), runs
# `verapdf --flavour ua1` on each, and prints pass/fail counts plus the names
# of any failures. Exits non-zero if anything failed.
#
# Usage:
#   scripts/agent/verapdf-corpus.sh [DIR]
#
# Env:
#   VERAPDF   path to the verapdf executable (default: `verapdf` on PATH)
#   JOBS      parallel validations (default: 4)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,15p' "$0"; exit 0
fi

scratch="$(ensure_scratch)"
corpus="${1:-$scratch/ua-corpus}"
verapdf="${VERAPDF:-verapdf}"
jobs="${JOBS:-4}"

if ! command -v "$verapdf" >/dev/null 2>&1; then
  echo "error: '$verapdf' not on PATH; set VERAPDF or install from https://verapdf.org/" >&2
  exit 2
fi
if [ ! -d "$corpus" ]; then
  echo "error: corpus dir not found: $corpus" >&2
  echo "       render PDF/UA fixtures into it first, e.g." >&2
  echo "       cargo test -p turbo-html2pdf-core --features pdf-ua -- --ignored emit_ua_corpus" >&2
  exit 2
fi

pdfs="$(find "$corpus" -type f -name '*.pdf' | sort)"
if [ -z "$pdfs" ]; then
  echo "no .pdf files in $corpus" >&2
  exit 2
fi
total=$(printf '%s\n' "$pdfs" | wc -l | tr -d ' ')
echo "veraPDF ua1: $total file(s) in $corpus (jobs=$jobs)"

tmp="$scratch/verapdf-$$"
mkdir -p "$tmp"
printf '%s\n' "$pdfs" | xargs -n1 -P "$jobs" -I{} sh -c '
  f="$1"; out="$2/$(basename "$1").xml"
  "$3" --flavour ua1 --format xml "$f" > "$out" 2>/dev/null || true
  compliant=$(grep -o "isCompliant=\"[a-z]*\"" "$out" | head -n1 | cut -d\" -f2)
  printf "%s\t%s\n" "${compliant:-error}" "$f"
' _ {} "$tmp" "$verapdf" > "$tmp/results.tsv"

pass=$(awk -F'\t' '$1=="true"{n++}END{print n+0}' "$tmp/results.tsv")
fail=$(awk -F'\t' '$1!="true"{n++}END{print n+0}' "$tmp/results.tsv")
echo "pass: $pass"
echo "fail: $fail"
if [ "$fail" -gt 0 ]; then
  echo "failures:"
  awk -F'\t' '$1!="true"{print "  " $1 "\t" $2}' "$tmp/results.tsv"
  exit 1
fi
