#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/scripts/build-release.sh"

require_text() {
  local expected="$1"
  if ! grep -Fq -- "$expected" "$SCRIPT"; then
    echo "build-release.sh is missing required text: $expected" >&2
    exit 1
  fi
}

reject_text() {
  local rejected="$1"
  if grep -Fq -- "$rejected" "$SCRIPT"; then
    echo "build-release.sh contains obsolete separate-build command: $rejected" >&2
    exit 1
  fi
}

require_text 'BUILD_CARGO_ARGS+=(--package y-cli --package y-gui --bins)'
require_text 'BUILD_CARGO_ARGS+=(--features y-gui/custom-protocol)'
require_text 'cargo "${BUILD_CARGO_ARGS[@]}"'
require_text 'TAURI_BUNDLE_ARGS=(bundle --ci)'
require_text 'TAURI_BUNDLE_ARGS+=(--bundles app)'
require_text 'npx @tauri-apps/cli "${TAURI_BUNDLE_ARGS[@]}"'
require_text 'package-macos-pkg.sh'
require_text 'cp "$BUNDLE_DIR"/pkg/*.pkg "$DIST_DIR/"'
require_text 'clean_release_metadata "$CLI_STAGING"'
require_text 'clean_release_metadata "$GUI_STAGING"'
require_text 'zip -X -r'
reject_text 'npx @tauri-apps/cli build'
reject_text '$BUNDLE_DIR/dmg/*.dmg'

echo "release build uses one shared Rust compilation and bundle-only packaging"
