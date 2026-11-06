#!/usr/bin/env bash
# Shared helpers for scripts/agent/*.sh.
# Sourced by the other scripts; not meant to run on its own.
set -euo pipefail

repo_root() {
  git -C "$(dirname "${BASH_SOURCE[0]}")/../.." rev-parse --show-toplevel
}

# Default scratchpad root: /tmp/turbo-html2pdf-agent-<user>. Agents may override
# via SCRATCH_ROOT. Kept outside the worktree so release-guard.sh does not see it.
scratch_root() {
  echo "${SCRATCH_ROOT:-/tmp/turbo-html2pdf-agent-${USER:-$(id -un)}}"
}

ensure_scratch() {
  local dir
  dir="$(scratch_root)"
  mkdir -p "$dir"
  echo "$dir"
}

# Chrome 156 headless-shell oracle (same path every sibling session uses).
chrome_headless_shell() {
  echo "${TURBO_CHROME_HEADLESS_SHELL:-/Users/grzegorz/.cache/turbo-consolidation/chrome156/chrome-headless-shell-mac-arm64/chrome-headless-shell}"
}

# rusty_v8 archive cache (so cold builds don't rebuild v8 bindings).
rusty_v8_archive_dir() {
  echo "${RUSTY_V8_ARCHIVE_DIR:-$HOME/.cache/turbo-consolidation/v8cache}"
}

print_help_if_asked() {
  for arg in "$@"; do
    case "$arg" in
      -h|--help|help) return 0 ;;
    esac
  done
  return 1
}
