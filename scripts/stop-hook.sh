#!/usr/bin/env bash
# Claude Code Stop hook: exit 2 sends the failure back to the agent.
set -uo pipefail
cd "$(dirname "$0")/.."
log=$(mktemp)
trap 'rm -f "$log"' EXIT

run() {
  if ! "$@" >"$log" 2>&1; then
    echo "check failed: $*" >&2
    tail -n 40 "$log" >&2
    exit 2
  fi
}

export SEXTANT_BUDGET_TESTS=1

run cargo fmt --all --check
run cargo clippy --workspace --all-targets -q -- -D warnings
run cargo test --workspace -q
run scripts/check-network.sh
