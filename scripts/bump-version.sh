#!/usr/bin/env bash
# bump-version.sh -- Update the project version across all manifest files.
#
# Usage:
#   ./scripts/bump-version.sh <new-version>
#   ./scripts/bump-version.sh 0.2.0
#   ./scripts/bump-version.sh --patch   (auto-increment patch)
#   ./scripts/bump-version.sh --minor   (auto-increment minor)
#   ./scripts/bump-version.sh --major   (auto-increment major)
#
# Files updated:
#   1. Cargo.toml               [workspace.package] version
#   2. crates/y-gui/package.json
#   3. crates/y-gui/src-tauri/tauri.conf.json
#   4. package.nix

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# -- Files that contain the version ----------------------------------------- #
CARGO_TOML="$REPO_ROOT/Cargo.toml"
PACKAGE_JSON="$REPO_ROOT/crates/y-gui/package.json"
TAURI_CONF="$REPO_ROOT/crates/y-gui/src-tauri/tauri.conf.json"
PACKAGE_NIX="$REPO_ROOT/package.nix"

# -- Helpers ---------------------------------------------------------------- #
die() { echo "ERROR: $*" >&2; exit 1; }

get_current_version() {
  # Read from the Single Source of Truth: Cargo.toml [workspace.package] version
  grep -E '^version\s*=' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

validate_semver() {
  local ver="$1"
  if ! echo "$ver" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    die "Invalid semver: $ver"
  fi
}

increment_version() {
  local current="$1" part="$2"
  local major minor patch
  IFS='.' read -r major minor patch <<< "${current%%-*}"

  case "$part" in
    major) major=$((major + 1)); minor=0; patch=0 ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    patch) patch=$((patch + 1)) ;;
    *) die "Unknown increment: $part" ;;
  esac

  echo "${major}.${minor}.${patch}"
}

# -- Main ------------------------------------------------------------------- #
if [ $# -ne 1 ]; then
  echo "Usage: $0 <new-version | --patch | --minor | --major>"
  exit 1
fi

CURRENT_VERSION="$(get_current_version)"
echo "Current version: $CURRENT_VERSION"

case "$1" in
  --patch) NEW_VERSION="$(increment_version "$CURRENT_VERSION" patch)" ;;
  --minor) NEW_VERSION="$(increment_version "$CURRENT_VERSION" minor)" ;;
  --major) NEW_VERSION="$(increment_version "$CURRENT_VERSION" major)" ;;
  *)       NEW_VERSION="$1" ;;
esac

validate_semver "$NEW_VERSION"

if [ "$NEW_VERSION" = "$CURRENT_VERSION" ]; then
  echo "Version is already $CURRENT_VERSION -- nothing to do."
  exit 0
fi

echo "Bumping version: $CURRENT_VERSION -> $NEW_VERSION"
echo ""

# 1. Cargo.toml -- [workspace.package] version
#    Match: version = "x.y.z" at beginning of line (under [workspace.package])
sed -i '' "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
echo "  [OK] Cargo.toml"

# 2. package.json -- "version": "x.y.z"
sed -i '' "s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" "$PACKAGE_JSON"
echo "  [OK] crates/y-gui/package.json"

# 3. tauri.conf.json -- "version": "x.y.z"
sed -i '' "s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" "$TAURI_CONF"
echo "  [OK] crates/y-gui/src-tauri/tauri.conf.json"

# 4. package.nix -- version = "x.y.z";
sed -i '' "s/version = \"$CURRENT_VERSION\";/version = \"$NEW_VERSION\";/" "$PACKAGE_NIX"
echo "  [OK] package.nix"

echo ""

# -- Verification ----------------------------------------------------------- #
ERRORS=0
verify() {
  local file="$1" pattern="$2" label="$3"
  if ! grep -q "$pattern" "$file"; then
    echo "  [FAIL] $label -- expected pattern not found: $pattern"
    ERRORS=$((ERRORS + 1))
  else
    echo "  [PASS] $label"
  fi
}

echo "Verifying..."
verify "$CARGO_TOML"    "version = \"$NEW_VERSION\""     "Cargo.toml"
verify "$PACKAGE_JSON"  "\"version\": \"$NEW_VERSION\""  "package.json"
verify "$TAURI_CONF"    "\"version\": \"$NEW_VERSION\""  "tauri.conf.json"
verify "$PACKAGE_NIX"   "version = \"$NEW_VERSION\";"    "package.nix"

echo ""
if [ "$ERRORS" -gt 0 ]; then
  die "Verification failed with $ERRORS error(s). Please check the files manually."
fi

echo "Done! Version bumped to $NEW_VERSION across all 4 files."
echo ""
echo "Next steps:"
echo "  git add -u"
echo "  git commit -m \"chore: bump version to $NEW_VERSION\""
echo "  git tag v$NEW_VERSION"
