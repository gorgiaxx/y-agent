#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BUILD_SCRIPT="$ROOT/crates/y-gui/src-tauri/build.rs"

if ! grep -Fq 'frontend_dist' "$BUILD_SCRIPT"; then
  echo "Tauri build script must track the frontend dist directory" >&2
  exit 1
fi

if ! grep -Fq 'cargo:rerun-if-changed={}' "$BUILD_SCRIPT"; then
  echo "Tauri build script must emit rerun-if-changed directives" >&2
  exit 1
fi

echo "Tauri binary rebuilds when frontend assets change"
