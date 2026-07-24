#!/usr/bin/env bash
# Build a macOS installer package containing the GUI application and CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_BUNDLE=""
CLI_BINARY=""
VERSION=""
PLATFORM=""
OUTPUT_DIR=""
PKGBUILD_BIN="${PKGBUILD_BIN:-pkgbuild}"
PACKAGE_SCRIPTS="$SCRIPT_DIR/macos/pkg-scripts"

usage() {
  cat <<'EOF'
Usage: package-macos-pkg.sh \
  --app-bundle PATH \
  --cli-binary PATH \
  --version VERSION \
  --platform darwin-arm64|darwin-amd64 \
  --output-dir PATH

Installs:
  /Applications/y-agent.app
  /usr/local/bin/yagent
  /usr/local/bin/y-agent -> yagent
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app-bundle)
      APP_BUNDLE="${2:-}"
      shift 2
      ;;
    --cli-binary)
      CLI_BINARY="${2:-}"
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --platform)
      PLATFORM="${2:-}"
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

if [[ -z "$APP_BUNDLE" || -z "$CLI_BINARY" || -z "$VERSION" || -z "$PLATFORM" || -z "$OUTPUT_DIR" ]]; then
  echo "error: all package arguments are required" >&2
  usage >&2
  exit 1
fi

if [[ ! -d "$APP_BUNDLE" ]]; then
  echo "error: application bundle not found: $APP_BUNDLE" >&2
  exit 1
fi
if [[ ! -x "$CLI_BINARY" ]]; then
  echo "error: CLI binary not found or not executable: $CLI_BINARY" >&2
  exit 1
fi
if [[ "$PLATFORM" != darwin-* ]]; then
  echo "error: macOS package platform must start with darwin-: $PLATFORM" >&2
  exit 1
fi
if ! command -v "$PKGBUILD_BIN" >/dev/null 2>&1; then
  echo "error: pkgbuild not found: $PKGBUILD_BIN" >&2
  exit 1
fi
if [[ ! -x "$PACKAGE_SCRIPTS/preinstall" ]]; then
  echo "error: package preinstall script is missing or not executable: $PACKAGE_SCRIPTS/preinstall" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

PAYLOAD_ROOT="$WORK_DIR/root"
mkdir -p "$PAYLOAD_ROOT/Applications" "$PAYLOAD_ROOT/usr/local/bin"

if command -v ditto >/dev/null 2>&1; then
  COPYFILE_DISABLE=1 ditto --norsrc --noextattr \
    "$APP_BUNDLE" "$PAYLOAD_ROOT/Applications/y-agent.app"
else
  COPYFILE_DISABLE=1 cp -R "$APP_BUNDLE" "$PAYLOAD_ROOT/Applications/y-agent.app"
fi

install -m 0755 "$CLI_BINARY" "$PAYLOAD_ROOT/usr/local/bin/yagent"
ln -s yagent "$PAYLOAD_ROOT/usr/local/bin/y-agent"

find "$PAYLOAD_ROOT" \( -name '.DS_Store' -o -name '._*' \) -delete
if command -v xattr >/dev/null 2>&1; then
  xattr -cr "$PAYLOAD_ROOT"
fi

OUTPUT_PKG="$OUTPUT_DIR/y-agent-${VERSION}-${PLATFORM}.pkg"
COPYFILE_DISABLE=1 "$PKGBUILD_BIN" \
  --root "$PAYLOAD_ROOT" \
  --scripts "$PACKAGE_SCRIPTS" \
  --ownership recommended \
  --identifier dev.y-agent.installer \
  --version "$VERSION" \
  --install-location / \
  "$OUTPUT_PKG"

echo "macOS installer written to $OUTPUT_PKG"
