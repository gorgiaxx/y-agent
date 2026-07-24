#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TAURI_CONFIG="$ROOT/crates/y-gui/src-tauri/tauri.conf.json"

node - "$TAURI_CONFIG" <<'EOF'
const fs = require('fs');

const config = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const mainWindow = config.app?.windows?.[0];

if (!mainWindow) {
  throw new Error('Tauri main window configuration is missing');
}

if (mainWindow.visible !== false) {
  throw new Error(
    'Tauri main window must remain hidden until native setup finishes',
  );
}
EOF

if ! grep -Fq 'main_window.show()?' "$ROOT/crates/y-gui/src-tauri/src/lib.rs"; then
  echo "native setup must show the main window without depending on React" >&2
  exit 1
fi

echo "Tauri main window is shown by native setup"
