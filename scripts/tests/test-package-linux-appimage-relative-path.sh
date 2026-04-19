#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TEST_ROOT="$REPO_ROOT/target/package-linux-appimage-test"
SOURCE_DIR="$TEST_ROOT/source"
BIN_DIR="$TEST_ROOT/bin"
OUTPUT_DIR="$TEST_ROOT/output"

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT

rm -rf "$TEST_ROOT"
mkdir -p "$SOURCE_DIR" "$BIN_DIR" "$OUTPUT_DIR"

SOURCE_APPIMAGE_ABS="$SOURCE_DIR/fake.AppImage"
APPIMAGETOOL_ABS="$BIN_DIR/fake-appimagetool"

cat > "$SOURCE_APPIMAGE_ABS" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--appimage-extract" ]]; then
  echo "unexpected arguments: $*" >&2
  exit 64
fi

mkdir -p squashfs-root
cat > squashfs-root/AppRun <<'APP'
#!/usr/bin/env bash
exit 0
APP
chmod 755 squashfs-root/AppRun
EOF
chmod 755 "$SOURCE_APPIMAGE_ABS"

cat > "$APPIMAGETOOL_ABS" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

appdir="$1"
output="$2"

if [[ ! -f "$appdir/AppRun" ]]; then
  echo "missing AppRun in $appdir" >&2
  exit 65
fi

printf 'patched appimage\n' > "$output"
EOF
chmod 755 "$APPIMAGETOOL_ABS"

SOURCE_APPIMAGE_REL="${SOURCE_APPIMAGE_ABS#$REPO_ROOT/}"
OUTPUT_DIR_REL="${OUTPUT_DIR#$REPO_ROOT/}"

(
  cd "$REPO_ROOT"
  APPIMAGETOOL="$APPIMAGETOOL_ABS" ./scripts/package-linux-appimage.sh \
    --source-appimage "$SOURCE_APPIMAGE_REL" \
    --output-dir "$OUTPUT_DIR_REL"
)

OUTPUT_APPIMAGE="$OUTPUT_DIR/$(basename "$SOURCE_APPIMAGE_ABS")"
if [[ ! -f "$OUTPUT_APPIMAGE" ]]; then
  echo "expected output AppImage at $OUTPUT_APPIMAGE" >&2
  exit 66
fi

grep -q 'patched appimage' "$OUTPUT_APPIMAGE"
