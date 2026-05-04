#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CI_WORKFLOW="$REPO_ROOT/.github/workflows/ci.yml"
RELEASE_WORKFLOW="$REPO_ROOT/.github/workflows/release.yml"
LOCAL_RELEASE_SCRIPT="$REPO_ROOT/scripts/build-release.sh"

require_text() {
  local file="$1"
  local expected="$2"

  if ! grep -Fq -- "$expected" "$file"; then
    echo "$(basename "$file") is missing required text: $expected" >&2
    exit 68
  fi
}

reject_text() {
  local file="$1"
  local rejected="$2"

  if grep -Fq -- "$rejected" "$file"; then
    echo "$(basename "$file") contains forbidden text: $rejected" >&2
    exit 69
  fi
}

require_text "$CI_WORKFLOW" "name: Rust"
require_text "$CI_WORKFLOW" "name: GUI"
require_text "$CI_WORKFLOW" "cache: npm"
require_text "$CI_WORKFLOW" "cache-dependency-path: crates/y-gui/package-lock.json"

require_text "$RELEASE_WORKFLOW" "node-version: \"22\""
require_text "$RELEASE_WORKFLOW" "cache: npm"
require_text "$RELEASE_WORKFLOW" "cache-dependency-path: crates/y-gui/package-lock.json"
require_text "$RELEASE_WORKFLOW" "run: npm ci"
reject_text "$RELEASE_WORKFLOW" "npm ci || npm install"
require_text "$RELEASE_WORKFLOW" "cache-targets: \${{ runner.os != 'Windows' }}"
require_text "$RELEASE_WORKFLOW" "cache-bin: \"false\""

require_text "$RELEASE_WORKFLOW" "Cache Arch package build inputs"
require_text "$RELEASE_WORKFLOW" ".cache/release/arch-pacman/pkg"
require_text "$RELEASE_WORKFLOW" ".cache/release/arch-cargo/registry"
require_text "$RELEASE_WORKFLOW" ".cache/release/arch-cargo/git"
require_text "$RELEASE_WORKFLOW" ".cache/release/arch-npm"
require_text "$RELEASE_WORKFLOW" '-v "$GITHUB_WORKSPACE:/work"'
reject_text "$RELEASE_WORKFLOW" "cp -a /work /home/builder/work"

require_text "$LOCAL_RELEASE_SCRIPT" '(cd "$GUI_DIR" && npm ci)'
reject_text "$LOCAL_RELEASE_SCRIPT" '(cd "$GUI_DIR" && npm install)'
