#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

require_text() {
  local file="$1"
  local pattern="$2"
  if ! grep -Fq -- "$pattern" "$file"; then
    echo "missing '$pattern' in $file" >&2
    exit 1
  fi
}

require_text "$ROOT/Cargo.toml" '"crates/y-xtask"'
require_text "$ROOT/.cargo/config.toml" 'app = "run --quiet -p y-xtask --"'
require_text "$ROOT/README.md" 'cargo app build'
require_text "$ROOT/README.md" 'cargo app cli -- --help'
require_text "$ROOT/README.md" 'cargo app gui'
require_text "$ROOT/crates/y-cli/Cargo.toml" 'default = ['
require_text "$ROOT/crates/y-cli/Cargo.toml" '"background_auto_wake"'
require_text "$ROOT/crates/y-cli/Cargo.toml" '"capability_packs"'
require_text "$ROOT/crates/y-cli/Cargo.toml" '"lsp"'
require_text "$ROOT/crates/y-gui/src-tauri/Cargo.toml" 'custom-protocol = ["tauri/custom-protocol"]'
require_text "$ROOT/crates/y-xtask/src/main.rs" '"--features", "y-gui/custom-protocol"'

echo "unified development entrypoints are configured"
