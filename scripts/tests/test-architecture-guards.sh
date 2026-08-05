#!/usr/bin/env bash
# Regression test for the architecture and quality guards.
#
# Verifies that `y-app guard all` actually fails on the conditions it claims to
# detect. A guard that only ever passes is worse than no guard, because it
# creates confidence without evidence.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG="$REPO_ROOT/guards.toml"
DOC="$REPO_ROOT/docs/standards/MEMORY_BUDGET.md"
BACKUP="$(mktemp)"
DOC_BACKUP="$(mktemp)"

cd "$REPO_ROOT"

cleanup() {
  cp "$BACKUP" "$CONFIG"
  cp "$DOC_BACKUP" "$DOC"
  rm -f "$BACKUP" "$DOC_BACKUP"
}
trap cleanup EXIT

cp "$CONFIG" "$BACKUP"
cp "$DOC" "$DOC_BACKUP"

run_guard() {
  cargo run -q -p y-xtask -- guard "$@" >/dev/null 2>&1
}

expect_pass() {
  local label="$1"
  shift
  if ! run_guard "$@"; then
    echo "guard should have passed: $label" >&2
    exit 67
  fi
}

expect_fail() {
  local label="$1"
  shift
  if run_guard "$@"; then
    echo "guard should have failed: $label" >&2
    exit 67
  fi
}

# The committed baseline must be clean.
expect_pass "committed baseline" all

# A budget lowered below the true count means the code regressed past its ceiling.
sed -i.bak 's/^panics = .*/panics = 0/' "$CONFIG" && rm -f "$CONFIG.bak"
expect_fail "budget exceeded" budgets
cp "$BACKUP" "$CONFIG"

# A budget raised above the true count means the ratchet was loosened; the guard
# must demand it be tightened back rather than accept the slack.
sed -i.bak 's/^panics = .*/panics = 100000/' "$CONFIG" && rm -f "$CONFIG.bak"
expect_fail "budget stale" budgets
cp "$BACKUP" "$CONFIG"

# An unknown budget key is a typo that would otherwise silently disable a metric.
printf '\n[budgets]\nponics = 1\n' >>"$CONFIG"
expect_fail "unknown budget key" budgets
cp "$BACKUP" "$CONFIG"

# Removing a recorded layering exception must surface the underlying violation.
sed -i.bak '/^"y-context" = /d' "$CONFIG" && rm -f "$CONFIG.bak"
expect_fail "layering violation" architecture
cp "$BACKUP" "$CONFIG"

# An exception for an edge that is not a violation is dead debt and must be
# reported, otherwise resolved debt accumulates in the file forever.
printf '"y-journal" = ["y-core"]\n' >>"$CONFIG"
expect_fail "stale layering exception" architecture
cp "$BACKUP" "$CONFIG"

# A documented ceiling that disagrees with its source declaration means the
# budget document has rotted; the guard exists precisely to catch that drift.
sed -i.bak 's/`5` | Concurrent toasts/`500` | Concurrent toasts/' "$DOC" && rm -f "$DOC.bak"
expect_fail "memory ceiling drift" memory
cp "$DOC_BACKUP" "$DOC"

# A documented constant that no longer exists must fail rather than be skipped.
printf '\n| `GONE_CEILING` | `crates/y-cli/src/tui/state.rs` | `1` | removed |\n' >>"$DOC"
expect_fail "missing documented constant" memory
cp "$DOC_BACKUP" "$DOC"

expect_pass "restored baseline" all

echo "architecture guard regression tests passed"
