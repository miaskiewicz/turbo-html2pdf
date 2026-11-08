#!/usr/bin/env bash
# test-all.sh — run every Rust gate the release runbook calls for.
#
# Order mirrors CLAUDE.md's pre-tag gate:
#   fmt --check -> clippy -D warnings -> cargo test -> cc-check (max 5) -> tarpaulin
#
# Usage:
#   scripts/agent/test-all.sh                        # whole workspace
#   scripts/agent/test-all.sh turbo-html2pdf-core    # one crate
#   scripts/agent/test-all.sh --skip tarpaulin
#
# Env:
#   SCRATCH_ROOT     base for CARGO_TARGET_DIR
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/_lib.sh"

if print_help_if_asked "$@"; then
  sed -n '2,15p' "$0"; exit 0
fi

crate=""
skip=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip) skip="$skip $2"; shift 2 ;;
    -*) echo "unknown arg: $1" >&2; exit 2 ;;
    *) crate="$1"; shift ;;
  esac
done

skipped() { case " $skip " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

root="$(repo_root)"
cd "$root"
scratch="$(ensure_scratch)"
export CARGO_TARGET_DIR="$scratch/target-test"
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

pkg_args=()
cc_target="crates"
if [ -n "$crate" ]; then
  pkg_args=(-p "$crate")
  case "$crate" in
    turbo-html2pdf-core) cc_target="crates/turbo-html2pdf-core" ;;
    turbo-pdf-napi)      cc_target="crates/turbo-pdf-napi" ;;
    turbo-pdf-py)        cc_target="crates/turbo-pdf-py" ;;
    turbo-pdf-wasm)      cc_target="crates/turbo-pdf-wasm" ;;
    turbo-html2pdf-mcp)  cc_target="crates/turbo-html2pdf-mcp" ;;
  esac
fi

echo "== cargo fmt --check =="
cargo fmt --all -- --check

if ! skipped clippy; then
  echo "== cargo clippy -D warnings =="
  if [ -n "$crate" ]; then
    cargo clippy "${pkg_args[@]}" --all-targets -- -D warnings
  else
    cargo clippy --workspace --all-targets -- -D warnings
  fi
fi

if ! skipped test; then
  echo "== cargo test =="
  if [ -n "$crate" ]; then
    cargo test "${pkg_args[@]}"
  else
    cargo test --workspace
  fi
fi

if ! skipped cc-check; then
  echo "== cc-check (cc <= 5) =="
  cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 "$cc_target"
fi

if ! skipped tarpaulin; then
  if command -v cargo-tarpaulin >/dev/null 2>&1; then
    echo "== cargo tarpaulin (100% gate) =="
    if [ -n "$crate" ]; then
      cargo tarpaulin -p "$crate"
    else
      cargo tarpaulin
    fi
  else
    echo "! cargo-tarpaulin not installed; skipping coverage (install with: cargo install cargo-tarpaulin)" >&2
  fi
fi

echo "✓ test-all complete${crate:+ for $crate}"
