#!/usr/bin/env bash
# =============================================================================
# build-release.sh -- Build y-agent and package into distributable zip archives
#
# Produces two zip files per platform:
#   y-agent-cli-{version}-{platform}.zip   CLI binary + config + skills + README
#   y-agent-gui-{version}-{platform}.zip   GUI installer (.dmg/.deb/.msi) + README
#
# Usage:
#   ./scripts/build-release.sh                                  Build CLI + GUI
#   ./scripts/build-release.sh cli                              Build CLI only
#   ./scripts/build-release.sh gui                              Build GUI only
#   ./scripts/build-release.sh --target aarch64-apple-darwin    Cross-compile
#   ./scripts/build-release.sh --version 1.2.3                  Override version
#
# Options:
#   cli              Build CLI binary only (skip GUI)
#   gui              Build GUI (Tauri) app only (skip CLI)
#   --target TRIPLE  Rust target triple for cross-compilation
#   --version VER    Override version string (default: read from Cargo.toml)
#   -h, --help       Show this help message
#
# Environment Variables:
#   SKIP_STRIP=1     Skip binary stripping (useful for debugging)
#
# Prerequisites:
#   - Rust toolchain (rustup, cargo)
#   - Node.js + npm (for GUI build)
#   - Platform-specific:
#     macOS:   Xcode Command Line Tools
#     Linux:   libwebkit2gtk-4.1-dev, libappindicator3-dev, librsvg2-dev, patchelf
#     Windows: Visual Studio Build Tools
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"

# -- Parse arguments -----------------------------------------------------------
BUILD_CLI=true
BUILD_GUI=true
BUILD_TARGET=""
VERSION_OVERRIDE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    cli)
      BUILD_CLI=true; BUILD_GUI=false; shift ;;
    gui)
      BUILD_CLI=false; BUILD_GUI=true; shift ;;
    --target)
      if [[ -z "${2:-}" ]]; then
        echo "error: --target requires a value (e.g. aarch64-apple-darwin)" >&2
        exit 1
      fi
      BUILD_TARGET="$2"; shift 2 ;;
    --version)
      if [[ -z "${2:-}" ]]; then
        echo "error: --version requires a value (e.g. 1.0.0)" >&2
        exit 1
      fi
      VERSION_OVERRIDE="$2"; shift 2 ;;
    -h|--help)
      # Print the comment block at the top of this file as help
      sed -n '/^# ====/,/^# ====/p' "$0" | sed 's/^# //' | sed 's/^#//'
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      echo "Run '$0 --help' for usage." >&2
      exit 1
      ;;
  esac
done

# -- Detect platform -----------------------------------------------------------
detect_platform() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    linux*)  os="linux" ;;
    darwin*) os="darwin" ;;
    msys*|mingw*|cygwin*) os="windows" ;;
    *)       echo "warning: unknown OS '${os}'" >&2; os="unknown" ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="amd64" ;;
    aarch64|arm64) arch="arm64" ;;
    *)             echo "warning: unknown arch '${arch}'" >&2 ;;
  esac

  echo "${os}-${arch}"
}

PLATFORM="$(detect_platform)"

# -- Version -------------------------------------------------------------------
if [[ -n "$VERSION_OVERRIDE" ]]; then
  VERSION="$VERSION_OVERRIDE"
else
  VERSION="$(grep -m1 'version' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"
fi

echo ""
echo "================================================================"
echo "  y-agent release build"
echo "================================================================"
echo "  Version:    ${VERSION}"
echo "  Platform:   ${PLATFORM}"
echo "  Build CLI:  ${BUILD_CLI}"
echo "  Build GUI:  ${BUILD_GUI}"
if [[ -n "$BUILD_TARGET" ]]; then
  echo "  Target:     ${BUILD_TARGET}"
fi
echo "  Output dir: ${DIST_DIR}/"
echo "================================================================"
echo ""

# -- Prepare dist directory ----------------------------------------------------
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# -- Build CLI -----------------------------------------------------------------
if [[ "$BUILD_CLI" == true ]]; then
  echo "[1/2] Building CLI binary..."

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
    echo "  Stripping binary..."
    strip "$CLI_BIN" 2>/dev/null || true
  fi

  # Package CLI zip
  CLI_ARCHIVE="y-agent-cli-${VERSION}-${PLATFORM}"
  CLI_STAGING="$DIST_DIR/$CLI_ARCHIVE"
  mkdir -p "$CLI_STAGING"

  cp "$CLI_BIN" "$CLI_STAGING/y-agent"
  cp -r "$PROJECT_ROOT/config" "$CLI_STAGING/config"
  cp -r "$PROJECT_ROOT/builtin-skills" "$CLI_STAGING/builtin-skills"
  cp "$PROJECT_ROOT/README.md" "$CLI_STAGING/"

  (cd "$DIST_DIR" && zip -r "${CLI_ARCHIVE}.zip" "$CLI_ARCHIVE")
  rm -rf "$CLI_STAGING"

  echo "  -> $DIST_DIR/${CLI_ARCHIVE}.zip ($(du -h "$DIST_DIR/${CLI_ARCHIVE}.zip" | cut -f1))"
  echo ""
fi

# -- Build GUI -----------------------------------------------------------------
if [[ "$BUILD_GUI" == true ]]; then
  if [[ "$BUILD_CLI" == true ]]; then
    echo "[2/2] Building GUI (Tauri) app..."
  else
    echo "[1/1] Building GUI (Tauri) app..."
  fi

  GUI_DIR="$PROJECT_ROOT/crates/y-gui"

  echo "  Installing npm dependencies..."
  (cd "$GUI_DIR" && npm install)

  echo "  Building Tauri app..."
  if [[ -n "$BUILD_TARGET" ]]; then
    (cd "$GUI_DIR" && npx @tauri-apps/cli build --target "$BUILD_TARGET")
  else
    (cd "$GUI_DIR" && npx @tauri-apps/cli build)
  fi

  # Tauri outputs to the workspace root target/ directory
  BUNDLE_DIR="$PROJECT_ROOT/target"
  if [[ -n "$BUILD_TARGET" ]]; then
    BUNDLE_DIR="$BUNDLE_DIR/$BUILD_TARGET"
  fi
  BUNDLE_DIR="$BUNDLE_DIR/release/bundle"

  # Unmount any DMGs that Tauri's create-dmg may have left mounted
  if [[ "$PLATFORM" == darwin-* ]]; then
    for vol in /Volumes/y-agent*; do
      if [[ -d "$vol" ]]; then
        echo "  Unmounting leftover volume: $vol"
        hdiutil detach "$vol" -quiet 2>/dev/null || true
      fi
    done
  fi

  # Package GUI zip
  GUI_ARCHIVE="y-agent-gui-${VERSION}-${PLATFORM}"
  GUI_STAGING="$DIST_DIR/$GUI_ARCHIVE"
  mkdir -p "$GUI_STAGING"

  case "$PLATFORM" in
    darwin-*)
      if compgen -G "$BUNDLE_DIR/dmg/*.dmg" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/dmg/*.dmg "$GUI_STAGING/"
        echo "  Collected .dmg"
      else
        echo "  WARNING: No .dmg found in $BUNDLE_DIR/dmg/"
      fi
      ;;
    linux-*)
      if compgen -G "$BUNDLE_DIR/deb/*.deb" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/deb/*.deb "$GUI_STAGING/"
        echo "  Collected .deb"
      fi
      if compgen -G "$BUNDLE_DIR/appimage/*.AppImage" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/appimage/*.AppImage "$GUI_STAGING/"
        echo "  Collected .AppImage"
      fi
      ;;
    windows-*)
      if compgen -G "$BUNDLE_DIR/msi/*.msi" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/msi/*.msi "$GUI_STAGING/"
        echo "  Collected .msi"
      fi
      if compgen -G "$BUNDLE_DIR/nsis/*.exe" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/nsis/*.exe "$GUI_STAGING/"
        echo "  Collected .exe"
      fi
      ;;
  esac

  cp "$PROJECT_ROOT/README.md" "$GUI_STAGING/"

  (cd "$DIST_DIR" && zip -r "${GUI_ARCHIVE}.zip" "$GUI_ARCHIVE")
  rm -rf "$GUI_STAGING"

  echo "  -> $DIST_DIR/${GUI_ARCHIVE}.zip ($(du -h "$DIST_DIR/${GUI_ARCHIVE}.zip" | cut -f1))"
  echo ""
fi

# -- Summary -------------------------------------------------------------------
echo "================================================================"
echo "  Build complete"
echo "================================================================"
echo ""
ls -lah "$DIST_DIR"/*.zip 2>/dev/null || echo "  (no zip files produced)"
echo ""
echo "================================================================"
