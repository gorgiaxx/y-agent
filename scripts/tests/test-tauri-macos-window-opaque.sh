#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TAURI_LIB="$ROOT/crates/y-gui/src-tauri/src/lib.rs"
TAURI_CONFIG="$ROOT/crates/y-gui/src-tauri/tauri.conf.json"
GLOBAL_CSS="$ROOT/crates/y-gui/src/styles/index.css"
APP_CSS="$ROOT/crates/y-gui/src/App.css"

if ! grep -Fq 'WindowEffect::Sidebar' "$TAURI_LIB"; then
  echo "macOS sidebar vibrancy must remain enabled" >&2
  exit 1
fi

node - "$TAURI_CONFIG" <<'EOF'
const fs = require('fs');
const config = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
if (config.app?.windows?.[0]?.transparent !== true) {
  throw new Error('macOS vibrancy requires a transparent native window');
}
EOF

if ! grep -Fq 'html[data-host="tauri"][data-platform="macos"] body' "$GLOBAL_CSS"; then
  echo "macOS Tauri body must expose the native vibrancy material" >&2
  exit 1
fi

if ! grep -Fq 'html[data-host="tauri"][data-platform="macos"] .app' "$APP_CSS"; then
  echo "macOS Tauri app root must expose the native vibrancy material" >&2
  exit 1
fi

if ! grep -A8 '^\.main-panel {' "$APP_CSS" | grep -Fq 'background: var(--surface-primary);'; then
  echo "main content panel must stay opaque above the sidebar vibrancy layer" >&2
  exit 1
fi

echo "macOS Tauri keeps sidebar vibrancy with an opaque main panel"
