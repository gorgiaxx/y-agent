#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORKFLOW="$REPO_ROOT/.github/workflows/ci.yml"

require_workflow_text() {
  local expected="$1"

  if ! grep -Fq "$expected" "$WORKFLOW"; then
    echo "CI workflow is missing required gate: $expected" >&2
    exit 67
  fi
}

require_workflow_text "working-directory: crates/y-gui"
require_workflow_text "npm ci"
require_workflow_text "npm test"
require_workflow_text "npm run lint"
require_workflow_text "npm run build"
require_workflow_text "npm run build:web"
require_workflow_text "working-directory: website"
require_workflow_text "corepack enable"
require_workflow_text "pnpm install --frozen-lockfile"
require_workflow_text "pnpm run build"
