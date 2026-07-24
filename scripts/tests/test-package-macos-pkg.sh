#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/scripts/package-macos-pkg.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

APP="$TMP_DIR/y-agent.app"
CLI="$TMP_DIR/y-agent"
OUTPUT_DIR="$TMP_DIR/output"
FAKE_PKGBUILD="$TMP_DIR/pkgbuild"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$OUTPUT_DIR"
printf 'gui' > "$APP/Contents/MacOS/y-gui"
printf 'finder metadata' > "$APP/Contents/Resources/.DS_Store"
printf 'appledouble metadata' > "$APP/Contents/Resources/._icon.icns"
printf '#!/bin/sh\nexit 0\n' > "$CLI"
chmod +x "$CLI"

cat > "$FAKE_PKGBUILD" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

ROOT=""
SCRIPTS=""
OUTPUT="${!#}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --root)
      ROOT="$2"
      shift 2
      ;;
    --scripts)
      SCRIPTS="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

test -d "$ROOT/Applications/y-agent.app"
test -x "$ROOT/usr/local/bin/yagent"
test "$(readlink "$ROOT/usr/local/bin/y-agent")" = "yagent"
test "${COPYFILE_DISABLE:-}" = "1"
if find "$ROOT" \( -name '.DS_Store' -o -name '._*' \) -print -quit | grep -q .; then
  echo "package payload contains macOS metadata files" >&2
  exit 1
fi
test -x "$SCRIPTS/preinstall"
grep -Fq 'chflags -R nouchg,noschg "/Applications/y-agent.app"' "$SCRIPTS/preinstall"
grep -Fq 'rm -rf "/Applications/y-agent.app"' "$SCRIPTS/preinstall"
printf 'package' > "$OUTPUT"
EOF
chmod +x "$FAKE_PKGBUILD"

PKGBUILD_BIN="$FAKE_PKGBUILD" "$SCRIPT" \
  --app-bundle "$APP" \
  --cli-binary "$CLI" \
  --version 1.2.3 \
  --platform darwin-arm64 \
  --output-dir "$OUTPUT_DIR"

test -f "$OUTPUT_DIR/y-agent-1.2.3-darwin-arm64.pkg"

echo "macOS package installs the app and yagent command"
