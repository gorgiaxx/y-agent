#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_LOCK="$REPO_ROOT/Cargo.lock"
TAURI_MANIFEST="$REPO_ROOT/crates/y-gui/src-tauri/Cargo.toml"

package_version() {
  local package="$1"

  awk -v package="$package" '
    $0 == "[[package]]" { in_package = 0 }
    $0 == "name = \"" package "\"" { in_package = 1 }
    in_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "$CARGO_LOCK"
}

major_minor() {
  local version="$1"
  printf '%s\n' "$version" | cut -d. -f1,2
}

tauri_version="$(package_version tauri)"
runtime_version="$(package_version tauri-runtime)"
runtime_wry_version="$(package_version tauri-runtime-wry)"

if [[ -z "$tauri_version" || -z "$runtime_version" || -z "$runtime_wry_version" ]]; then
  echo "failed to read Tauri package versions from Cargo.lock" >&2
  exit 70
fi

tauri_family="$(major_minor "$tauri_version")"
runtime_family="$(major_minor "$runtime_version")"
runtime_wry_family="$(major_minor "$runtime_wry_version")"

if [[ "$tauri_family" != "$runtime_family" || "$tauri_family" != "$runtime_wry_family" ]]; then
  echo "Tauri crate family mismatch: tauri=$tauri_version tauri-runtime=$runtime_version tauri-runtime-wry=$runtime_wry_version" >&2
  exit 71
fi

if grep -Fq 'tauri = { version = ">=2.10, <2.11"' "$TAURI_MANIFEST"; then
  echo "Tauri manifest must not pin tauri below the runtime minor version" >&2
  exit 72
fi
