#!/usr/bin/env bash
# =============================================================================
# build-release.sh — Build y-agent CLI + GUI and package into a zip archive
#
# Usage:
#   ./scripts/build-release.sh              # Build all (CLI + GUI)
#   ./scripts/build-release.sh cli          # Build CLI only
#   ./scripts/build-release.sh gui          # Build GUI only
#   ./scripts/build-release.sh --target aarch64-apple-darwin   # Cross-compile
#
# Environment Variables:
#   BUILD_TARGET   - Rust target triple (default: host)
#   SKIP_STRIP     - Set to "1" to skip binary stripping
#   VERSION        - Override version string (default: from Cargo.toml)
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"

# ── Parse arguments ───────────────────────────────────────────────────────────
BUILD_CLI=false
BUILD_GUI=false
BUILD_TARGET=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    cli)      BUILD_CLI=true; shift ;;
    gui)      BUILD_GUI=true; shift ;;
    --target) BUILD_TARGET="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [cli|gui] [--target <triple>]"
      echo "  cli            Build CLI binary only"
      echo "  gui            Build GUI (Tauri) app only"
      echo "  --target       Specify Rust target triple"
      echo ""
      echo "If no component specified, builds both cli and gui."
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

# Default: build both
if [[ "$BUILD_CLI" == false && "$BUILD_GUI" == false ]]; then
  BUILD_CLI=true
  BUILD_GUI=true
fi

# ── Detect platform ──────────────────────────────────────────────────────────
detect_platform() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    linux*)  os="linux" ;;
    darwin*) os="darwin" ;;
    msys*|mingw*|cygwin*) os="windows" ;;
    *)       os="unknown" ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="amd64" ;;
    aarch64|arm64) arch="arm64" ;;
    *)             arch="$arch" ;;
  esac

  echo "${os}-${arch}"
}

PLATFORM="$(detect_platform)"

if [[ -z "$BUILD_TARGET" ]]; then
  BUILD_TARGET="${BUILD_TARGET:-}"
fi

# ── Version ──────────────────────────────────────────────────────────────────
if [[ -z "${VERSION:-}" ]]; then
  VERSION="$(grep -m1 'version' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"
fi
echo "📦 Building y-agent v${VERSION} for ${PLATFORM}"

# ── Prepare dist directory ───────────────────────────────────────────────────
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# ── Build CLI ────────────────────────────────────────────────────────────────
if [[ "$BUILD_CLI" == true ]]; then
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  🔨 Building CLI binary"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  CLI_CARGO_ARGS=(build --release --bin y-agent)
  if [[ -n "$BUILD_TARGET" ]]; then
    CLI_CARGO_ARGS+=(--target "$BUILD_TARGET")
  fi

  (cd "$PROJECT_ROOT" && cargo "${CLI_CARGO_ARGS[@]}")

  # Locate binary
  if [[ -n "$BUILD_TARGET" ]]; then
    CLI_BIN="$PROJECT_ROOT/target/$BUILD_TARGET/release/y-agent"
  else
    CLI_BIN="$PROJECT_ROOT/target/release/y-agent"
  fi

  # Strip binary
  if [[ "${SKIP_STRIP:-0}" != "1" && -f "$CLI_BIN" ]]; then
    echo "  🔧 Stripping binary..."
    strip "$CLI_BIN" 2>/dev/null || true
  fi

  # Prepare CLI distribution
  CLI_DIST="$DIST_DIR/y-agent-cli-${VERSION}-${PLATFORM}"
  mkdir -p "$CLI_DIST"
  cp "$CLI_BIN" "$CLI_DIST/y-agent"
  cp -r "$PROJECT_ROOT/config" "$CLI_DIST/config"
  cp -r "$PROJECT_ROOT/builtin-skills" "$CLI_DIST/builtin-skills"
  cp "$PROJECT_ROOT/README.md" "$CLI_DIST/"

  # Create zip
  (cd "$DIST_DIR" && zip -r "y-agent-cli-${VERSION}-${PLATFORM}.zip" "$(basename "$CLI_DIST")")
  rm -rf "$CLI_DIST"

  CLI_ZIP="$DIST_DIR/y-agent-cli-${VERSION}-${PLATFORM}.zip"
  echo "  ✅ CLI zip: $CLI_ZIP ($(du -h "$CLI_ZIP" | cut -f1))"
fi

# ── Build GUI ────────────────────────────────────────────────────────────────
if [[ "$BUILD_GUI" == true ]]; then
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  🖥️  Building GUI (Tauri) app"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  GUI_DIR="$PROJECT_ROOT/crates/y-gui"

  # Install frontend dependencies
  echo "  📥 Installing npm dependencies..."
  (cd "$GUI_DIR" && npm install)

  # Build with Tauri
  echo "  🏗️  Building Tauri app..."
  TAURI_ARGS=()
  if [[ -n "$BUILD_TARGET" ]]; then
    TAURI_ARGS+=(--target "$BUILD_TARGET")
  fi

  (cd "$GUI_DIR" && npx @tauri-apps/cli build "${TAURI_ARGS[@]}")

  # Collect outputs
  GUI_DIST="$DIST_DIR/y-agent-gui-${VERSION}-${PLATFORM}"
  mkdir -p "$GUI_DIST"

  # Copy platform-specific bundle outputs
  BUNDLE_DIR="$GUI_DIR/src-tauri/target"
  if [[ -n "$BUILD_TARGET" ]]; then
    BUNDLE_DIR="$BUNDLE_DIR/$BUILD_TARGET"
  fi
  BUNDLE_DIR="$BUNDLE_DIR/release/bundle"

  case "$PLATFORM" in
    darwin-*)
      # Copy .dmg and .app
      if compgen -G "$BUNDLE_DIR/dmg/*.dmg" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/dmg/*.dmg "$GUI_DIST/"
      fi
      if compgen -G "$BUNDLE_DIR/macos/*.app" > /dev/null 2>&1; then
        cp -r "$BUNDLE_DIR"/macos/*.app "$GUI_DIST/"
      fi
      ;;
    linux-*)
      # Copy .deb, .AppImage
      if compgen -G "$BUNDLE_DIR/deb/*.deb" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/deb/*.deb "$GUI_DIST/"
      fi
      if compgen -G "$BUNDLE_DIR/appimage/*.AppImage" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/appimage/*.AppImage "$GUI_DIST/"
      fi
      ;;
    windows-*)
      # Copy .msi, .exe
      if compgen -G "$BUNDLE_DIR/msi/*.msi" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/msi/*.msi "$GUI_DIST/"
      fi
      if compgen -G "$BUNDLE_DIR/nsis/*.exe" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/nsis/*.exe "$GUI_DIST/"
      fi
      ;;
  esac

  cp "$PROJECT_ROOT/README.md" "$GUI_DIST/"

  # Create zip
  (cd "$DIST_DIR" && zip -r "y-agent-gui-${VERSION}-${PLATFORM}.zip" "$(basename "$GUI_DIST")")
  rm -rf "$GUI_DIST"

  GUI_ZIP="$DIST_DIR/y-agent-gui-${VERSION}-${PLATFORM}.zip"
  echo "  ✅ GUI zip: $GUI_ZIP ($(du -h "$GUI_ZIP" | cut -f1))"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  📋 Build Summary"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Version:  ${VERSION}"
echo "  Platform: ${PLATFORM}"
echo "  Output:   ${DIST_DIR}/"
echo ""
ls -lah "$DIST_DIR"/*.zip 2>/dev/null || echo "  (no zip files produced)"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
