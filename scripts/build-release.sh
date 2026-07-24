#!/usr/bin/env bash
# =============================================================================
# build-release.sh -- Build y-agent and package into distributable zip archives
#
# Produces two zip files per platform from one shared Rust compilation:
#   y-agent-cli-{version}-{platform}.zip   CLI binary + config + skills + README
#   y-agent-gui-{version}-{platform}.zip   GUI installer bundle + README
#
# macOS GUI archives include a .pkg installer that upgrades
# /Applications/y-agent.app and installs /usr/local/bin/yagent.
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
#              appimagetool (for patched AppImage output)
#              makepkg       (optional, for pacman/Arch package output)
#     Windows: Visual Studio Build Tools
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"

clean_release_metadata() {
  local path="$1"
  find "$path" \( -name '.DS_Store' -o -name '._*' \) -delete
  if command -v xattr >/dev/null 2>&1; then
    xattr -cr "$path"
  fi
}

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
detect_host_os() {
  local os
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  case "$os" in
    linux*)  echo "linux" ;;
    darwin*) echo "darwin" ;;
    msys*|mingw*|cygwin*) echo "windows" ;;
    *)       echo "unknown" ;;
  esac
}

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

# Derive platform string from a Rust target triple
platform_from_target() {
  local target="$1" os arch
  case "$target" in
    *-linux-*)   os="linux" ;;
    *-apple-*)   os="darwin" ;;
    *-windows-*) os="windows" ;;
    *)           os="unknown" ;;
  esac
  case "$target" in
    x86_64-*)  arch="amd64" ;;
    aarch64-*) arch="arm64" ;;
    i686-*)    arch="x86" ;;
    *)         arch="unknown" ;;
  esac
  echo "${os}-${arch}"
}

HOST_OS="$(detect_host_os)"

# When --target is used, derive PLATFORM from the target triple;
# otherwise detect from the current host.
if [[ -n "$BUILD_TARGET" ]]; then
  PLATFORM="$(platform_from_target "$BUILD_TARGET")"
else
  PLATFORM="$(detect_platform)"
fi

# -- Validate cross-compilation feasibility ------------------------------------
if [[ -n "$BUILD_TARGET" ]]; then
  TARGET_OS=""
  case "$BUILD_TARGET" in
    *-windows-msvc*) TARGET_OS="windows-msvc" ;;
    *-windows-gnu*)  TARGET_OS="windows-gnu" ;;
    *-linux-*)       TARGET_OS="linux" ;;
    *-apple-*)       TARGET_OS="darwin" ;;
  esac

  # MSVC targets require the Windows SDK + MSVC linker -- only available on Windows
  if [[ "$TARGET_OS" == "windows-msvc" && "$HOST_OS" != "windows" ]]; then
    echo "error: cannot cross-compile to '$BUILD_TARGET' from $HOST_OS." >&2
    echo "" >&2
    echo "  The -msvc target requires the Microsoft Visual C++ linker and" >&2
    echo "  Windows SDK, which are only available on Windows." >&2
    echo "" >&2
    echo "  Alternatives:" >&2
    echo "    1. Build on a Windows machine or Windows CI runner." >&2
    echo "    2. Use the GNU target (requires mingw-w64 toolchain):" >&2
    echo "       ./scripts/build-release.sh --target x86_64-pc-windows-gnu" >&2
    echo "" >&2
    exit 1
  fi

  # Check that the Rust target is installed via rustup
  if ! rustup target list --installed | grep -q "^${BUILD_TARGET}$"; then
    echo "error: Rust target '$BUILD_TARGET' is not installed." >&2
    echo "" >&2
    echo "  Install it with:" >&2
    echo "    rustup target add $BUILD_TARGET" >&2
    echo "" >&2
    exit 1
  fi

  # Windows GNU targets from macOS/Linux need the mingw-w64 cross-linker
  if [[ "$TARGET_OS" == "windows-gnu" && "$HOST_OS" != "windows" ]]; then
    MINGW_CC=""
    case "$BUILD_TARGET" in
      x86_64-*)  MINGW_CC="x86_64-w64-mingw32-gcc" ;;
      i686-*)    MINGW_CC="i686-w64-mingw32-gcc" ;;
      aarch64-*) MINGW_CC="aarch64-w64-mingw32-gcc" ;;
    esac
    if [[ -n "$MINGW_CC" ]] && ! command -v "$MINGW_CC" &>/dev/null; then
      echo "error: MinGW-w64 cross-compiler '$MINGW_CC' not found." >&2
      echo "" >&2
      if [[ "$HOST_OS" == "darwin" ]]; then
        echo "  Install it with:" >&2
        echo "    brew install mingw-w64" >&2
      else
        echo "  Install it with:" >&2
        echo "    sudo apt-get install mingw-w64      # Debian/Ubuntu" >&2
        echo "    sudo dnf install mingw64-gcc         # Fedora" >&2
      fi
      echo "" >&2
      exit 1
    fi

    # Tauri Windows build needs NSIS to bundle the installer
    if [[ "$BUILD_GUI" == true ]] && ! command -v makensis &>/dev/null; then
      echo "error: Tauri GUI bundle for Windows requires NSIS, but 'makensis' not found." >&2
      echo "" >&2
      if [[ "$HOST_OS" == "darwin" ]]; then
        echo "  Install it with:" >&2
        echo "    brew install nsis" >&2
      else
        echo "  Install it with:" >&2
        echo "    sudo apt-get install nsis" >&2
      fi
      echo "" >&2
      exit 1
    fi
  fi

  # Linux targets from macOS require a cross-linker (give a hint)
  if [[ "$TARGET_OS" == "linux" && "$HOST_OS" == "darwin" ]]; then
    echo "warning: cross-compiling to Linux from macOS requires a cross-linker" >&2
    echo "         (e.g. install via: brew install filosottile/musl-cross/musl-cross)" >&2
    echo "" >&2
  fi
fi

# Binary extension for the target
BIN_EXT=""
case "${BUILD_TARGET:-}" in
  *-windows-*) BIN_EXT=".exe" ;;
esac

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

# -- Determine target release directory ----------------------------------------
if [[ -n "$BUILD_TARGET" ]]; then
  TARGET_RELEASE_DIR="$PROJECT_ROOT/target/$BUILD_TARGET/release"
else
  TARGET_RELEASE_DIR="$PROJECT_ROOT/target/release"
fi

# -- Prepare shared build ------------------------------------------------------
GUI_DIR="$PROJECT_ROOT/crates/y-gui"
BUNDLE_DIR="$TARGET_RELEASE_DIR/bundle"
NEED_CLI_BINARY="$BUILD_CLI"

# The macOS GUI installer always includes the CLI so `yagent` is available
# immediately after installation, even in GUI-only release mode.
if [[ "$BUILD_GUI" == true && "$PLATFORM" == darwin-* ]]; then
  NEED_CLI_BINARY=true
fi

if [[ "$BUILD_GUI" == true ]]; then
  echo "Preparing shared frontend..."
  rm -rf "$BUNDLE_DIR"
  (cd "$GUI_DIR" && npm ci)
  (cd "$GUI_DIR" && npm run build)
  echo ""
fi

echo "Building release binaries in one Cargo invocation..."
if [[ "$NEED_CLI_BINARY" == true ]]; then
  rm -f "$TARGET_RELEASE_DIR/y-agent${BIN_EXT}"
fi
if [[ "$BUILD_GUI" == true ]]; then
  rm -f "$TARGET_RELEASE_DIR/y-gui${BIN_EXT}"
fi

BUILD_CARGO_ARGS=(build --release)
if [[ "$NEED_CLI_BINARY" == true && "$BUILD_GUI" == true ]]; then
  BUILD_CARGO_ARGS+=(--package y-cli --package y-gui --bins)
elif [[ "$NEED_CLI_BINARY" == true ]]; then
  BUILD_CARGO_ARGS+=(--package y-cli --bin y-agent)
else
  BUILD_CARGO_ARGS+=(--package y-gui --bin y-gui)
fi
if [[ "$BUILD_GUI" == true ]]; then
  # Tauri uses this feature to embed frontendDist instead of loading devUrl.
  BUILD_CARGO_ARGS+=(--features y-gui/custom-protocol)
fi
if [[ -n "$BUILD_TARGET" ]]; then
  BUILD_CARGO_ARGS+=(--target "$BUILD_TARGET")
fi
(cd "$PROJECT_ROOT" && cargo "${BUILD_CARGO_ARGS[@]}")

CLI_BIN="$TARGET_RELEASE_DIR/y-agent${BIN_EXT}"
if [[ "$NEED_CLI_BINARY" == true && "${SKIP_STRIP:-0}" != "1" && -f "$CLI_BIN" && -z "$BIN_EXT" ]]; then
  echo "  Stripping CLI binary..."
  strip "$CLI_BIN" 2>/dev/null || true
fi
echo ""

# -- Package CLI ---------------------------------------------------------------
if [[ "$BUILD_CLI" == true ]]; then
  echo "Packaging CLI archive..."
  CLI_ARCHIVE="y-agent-cli-${VERSION}-${PLATFORM}"
  CLI_STAGING="$DIST_DIR/$CLI_ARCHIVE"
  mkdir -p "$CLI_STAGING"

  cp "$CLI_BIN" "$CLI_STAGING/y-agent${BIN_EXT}"
  cp -r "$PROJECT_ROOT/config" "$CLI_STAGING/config"
  cp -r "$PROJECT_ROOT/skills" "$CLI_STAGING/skills"
  cp "$PROJECT_ROOT/README.md" "$CLI_STAGING/"

  clean_release_metadata "$CLI_STAGING"
  (cd "$DIST_DIR" && zip -X -r "${CLI_ARCHIVE}.zip" "$CLI_ARCHIVE")
  rm -rf "$CLI_STAGING"

  echo "  -> $DIST_DIR/${CLI_ARCHIVE}.zip ($(du -h "$DIST_DIR/${CLI_ARCHIVE}.zip" | cut -f1))"
  echo ""
fi

# -- Bundle GUI ---------------------------------------------------------------
if [[ "$BUILD_GUI" == true ]]; then
  echo "Bundling prebuilt GUI binary..."
  TAURI_BUNDLE_ARGS=(bundle --ci)
  if [[ "$PLATFORM" == darwin-* ]]; then
    # The .pkg is the canonical macOS installer. Bundling only the .app avoids
    # Finder permission conflicts from drag-and-drop DMG upgrades and keeps
    # create-dmg failures from blocking the combined release build.
    TAURI_BUNDLE_ARGS+=(--bundles app)
  fi
  if [[ -n "$BUILD_TARGET" ]]; then
    TAURI_BUNDLE_ARGS+=(--target "$BUILD_TARGET")
  fi
  (cd "$GUI_DIR" && npx @tauri-apps/cli "${TAURI_BUNDLE_ARGS[@]}")

  if [[ "$PLATFORM" == linux-* ]]; then
    if compgen -G "$BUNDLE_DIR/appimage/*.AppImage" > /dev/null 2>&1; then
      if [[ -n "${APPIMAGETOOL:-}" ]] || command -v appimagetool >/dev/null 2>&1; then
        echo "  Patching AppImage for Wayland-compatible launch..."
        PATCHED_APPIMAGE_DIR="$BUNDLE_DIR/appimage-patched"
        mkdir -p "$PATCHED_APPIMAGE_DIR"
        ORIGINAL_APPIMAGE="$(find "$BUNDLE_DIR/appimage" -maxdepth 1 -name '*.AppImage' | head -1)"
        "$PROJECT_ROOT/scripts/package-linux-appimage.sh" \
          --source-appimage "$ORIGINAL_APPIMAGE" \
          --output-dir "$PATCHED_APPIMAGE_DIR"
        rm -f "$BUNDLE_DIR"/appimage/*.AppImage
        cp "$PATCHED_APPIMAGE_DIR"/*.AppImage "$BUNDLE_DIR/appimage/"
      else
        echo "  WARNING: appimagetool not found; keeping unpatched AppImage"
      fi
    fi

    if command -v makepkg >/dev/null 2>&1; then
      echo "  Building pacman package..."
      mkdir -p "$BUNDLE_DIR/pacman"
      "$PROJECT_ROOT/scripts/package-linux-pacman.sh" \
        --version "$VERSION" \
        --binary-path "$TARGET_RELEASE_DIR/y-gui" \
        --output-dir "$BUNDLE_DIR/pacman"
    else
      echo "  NOTE: makepkg not found; skipping pacman package"
    fi
  fi

  if [[ "$PLATFORM" == darwin-* && "$HOST_OS" == "darwin" ]]; then
    MACOS_APP_BUNDLE="$BUNDLE_DIR/macos/y-agent.app"
    if [[ -d "$MACOS_APP_BUNDLE" ]]; then
      mkdir -p "$BUNDLE_DIR/pkg"
      "$PROJECT_ROOT/scripts/package-macos-pkg.sh" \
        --app-bundle "$MACOS_APP_BUNDLE" \
        --cli-binary "$CLI_BIN" \
        --version "$VERSION" \
        --platform "$PLATFORM" \
        --output-dir "$BUNDLE_DIR/pkg"
    else
      echo "  WARNING: No .app found at $MACOS_APP_BUNDLE; skipping .pkg installer" >&2
    fi
  fi

  # Package GUI zip
  GUI_ARCHIVE="y-agent-gui-${VERSION}-${PLATFORM}"
  GUI_STAGING="$DIST_DIR/$GUI_ARCHIVE"
  mkdir -p "$GUI_STAGING"

  case "$PLATFORM" in
    darwin-*)
      if compgen -G "$BUNDLE_DIR/pkg/*.pkg" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/pkg/*.pkg "$GUI_STAGING/"
        cp "$BUNDLE_DIR"/pkg/*.pkg "$DIST_DIR/"
        echo "  Collected .pkg installer (GUI + yagent CLI)"
      else
        echo "  WARNING: No .pkg found in $BUNDLE_DIR/pkg/"
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
      if compgen -G "$BUNDLE_DIR/pacman/*.pkg.tar.zst" > /dev/null 2>&1; then
        cp "$BUNDLE_DIR"/pacman/*.pkg.tar.zst "$GUI_STAGING/"
        echo "  Collected .pkg.tar.zst"
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

  clean_release_metadata "$GUI_STAGING"
  (cd "$DIST_DIR" && zip -X -r "${GUI_ARCHIVE}.zip" "$GUI_ARCHIVE")
  rm -rf "$GUI_STAGING"

  echo "  -> $DIST_DIR/${GUI_ARCHIVE}.zip ($(du -h "$DIST_DIR/${GUI_ARCHIVE}.zip" | cut -f1))"
  echo ""
fi

# -- Summary -------------------------------------------------------------------
echo "================================================================"
echo "  Build complete"
echo "================================================================"
echo ""
RELEASE_FILES=("$DIST_DIR"/*)
if [[ -e "${RELEASE_FILES[0]}" ]]; then
  ls -lah "${RELEASE_FILES[@]}"
else
  echo "  (no release files produced)"
fi
echo ""
echo "================================================================"
