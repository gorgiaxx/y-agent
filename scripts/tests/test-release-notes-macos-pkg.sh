#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/dist" "$TMP_DIR/bin"
touch "$TMP_DIR/dist/y-agent-1.2.3-darwin-arm64.pkg"

for command_name in .pkg yagent y-agent; do
  cat > "$TMP_DIR/bin/$command_name" <<EOF
#!/usr/bin/env bash
touch "$TMP_DIR/unexpected-command-substitution"
EOF
  chmod +x "$TMP_DIR/bin/$command_name"
done

PATH="$TMP_DIR/bin:$PATH" \
VERSION=1.2.3 \
DIST_DIR="$TMP_DIR/dist" \
OUTPUT_FILE="$TMP_DIR/release-notes.md" \
  "$ROOT/.github/scripts/generate-release-notes.sh" >/dev/null

test ! -e "$TMP_DIR/unexpected-command-substitution"
grep -Fq 'PKG installer + CLI' "$TMP_DIR/release-notes.md"
grep -Fq '`yagent`' "$TMP_DIR/release-notes.md"

echo "macOS PKG release notes do not execute Markdown code spans"
