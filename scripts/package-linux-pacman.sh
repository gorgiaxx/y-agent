#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PKGBUILD_TEMPLATE="$REPO_ROOT/packaging/linux/pacman/PKGBUILD.in"
DESKTOP_FILE="$REPO_ROOT/packaging/linux/pacman/y-agent.desktop"
WRAPPER_FILE="$REPO_ROOT/packaging/linux/pacman/y-gui"
ICON_DIR="$REPO_ROOT/crates/y-gui/src-tauri/icons"

VERSION=""
PKGREL="1"
OUTPUT_DIR=""
BINARY_PATH=""

usage() {
  cat <<'EOF'
Usage:
  ./scripts/package-linux-pacman.sh \
    --version 0.4.0 \
    --binary-path /path/to/y-gui \
    --output-dir /path/to/dist

Options:
  --version VER         Package version.
  --pkgrel REL          Package release number (default: 1).
  --binary-path PATH    Built y-gui binary to package.
  --output-dir DIR      Directory for resulting .pkg.tar.zst.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --pkgrel)
      PKGREL="${2:-}"
      shift 2
      ;;
    --binary-path)
      BINARY_PATH="${2:-}"
      shift 2
      ;;
    --output-dir)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$VERSION" || -z "$OUTPUT_DIR" ]]; then
  echo "error: --version and --output-dir are required" >&2
  usage >&2
  exit 1
fi

if [[ -z "$BINARY_PATH" ]]; then
  BINARY_PATH="$REPO_ROOT/target/release/y-gui"
fi

if [[ ! -f "$BINARY_PATH" ]]; then
  echo "error: binary not found: $BINARY_PATH" >&2
  exit 1
fi

if ! command -v makepkg >/dev/null 2>&1; then
  echo "error: makepkg is required to build pacman packages" >&2
  exit 1
fi

if [[ ! -f "$PKGBUILD_TEMPLATE" || ! -f "$DESKTOP_FILE" || ! -f "$WRAPPER_FILE" ]]; then
  echo "error: pacman packaging assets are missing" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

WORK_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

cp "$BINARY_PATH" "$WORK_DIR/y-gui-bin"
cp "$WRAPPER_FILE" "$WORK_DIR/y-gui-wrapper"
cp "$DESKTOP_FILE" "$WORK_DIR/y-agent.desktop"
cp "$ICON_DIR/32x32.png" "$WORK_DIR/32x32.png"
cp "$ICON_DIR/128x128.png" "$WORK_DIR/128x128.png"
cp "$ICON_DIR/128x128@2x.png" "$WORK_DIR/128x128@2x.png"
tar -cf "$WORK_DIR/skills.tar" -C "$REPO_ROOT" skills
chmod 755 "$WORK_DIR/y-gui-bin" "$WORK_DIR/y-gui-wrapper"

sed \
  -e "s/@VERSION@/$VERSION/g" \
  -e "s/@PKGREL@/$PKGREL/g" \
  "$PKGBUILD_TEMPLATE" > "$WORK_DIR/PKGBUILD"

(
  cd "$WORK_DIR"
  makepkg --nodeps --force --clean
)

find "$WORK_DIR" -maxdepth 1 -type f -name '*.pkg.tar.zst' -exec mv {} "$OUTPUT_DIR/" \;

echo "Pacman package written to $OUTPUT_DIR"
