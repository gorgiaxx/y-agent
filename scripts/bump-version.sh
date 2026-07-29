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
#   5. crates/y-gui/package-lock.json  (top-level and root package version only)
#   6. Cargo.lock                 (workspace crate versions, via cargo metadata)
#
# Note: the lock file is edited in place instead of running
# `npm install --package-lock-only`, because locally installed npm versions
# newer than the CI toolchain (Node 22 / npm 10) rewrite unrelated lock
# metadata (e.g. `peer` flags), which then fails the CI lock-sync check.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# -- Files that contain the version ----------------------------------------- #
CARGO_TOML="$REPO_ROOT/Cargo.toml"
PACKAGE_JSON="$REPO_ROOT/crates/y-gui/package.json"
TAURI_CONF="$REPO_ROOT/crates/y-gui/src-tauri/tauri.conf.json"
PACKAGE_NIX="$REPO_ROOT/package.nix"
PACKAGE_LOCK="$REPO_ROOT/crates/y-gui/package-lock.json"

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

# 5. package-lock.json -- "version": "x.y.z"
#    Only the top-level "version" and the root packages[""]."version" entries
#    (both within the first lines) belong to this project; dependency entries
#    further down must not be touched.
sed -i '' "1,20s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" "$PACKAGE_LOCK"
echo "  [OK] crates/y-gui/package-lock.json"

# 6. Cargo.lock -- workspace crate versions
#    `cargo metadata` re-resolves and rewrites the lock file to match the
#    manifests while keeping existing dependency pins (unlike `cargo update`).
( cd "$REPO_ROOT" && cargo metadata --format-version 1 > /dev/null )
echo "  [OK] Cargo.lock"

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
if head -20 "$PACKAGE_LOCK" | grep -q "\"version\": \"$CURRENT_VERSION\""; then
  echo "  [FAIL] package-lock.json -- old version still present in header"
  ERRORS=$((ERRORS + 1))
else
  echo "  [PASS] package-lock.json"
fi
if grep -A1 '^name = "y-agent"$' "$REPO_ROOT/Cargo.lock" | grep -q "version = \"$NEW_VERSION\""; then
  echo "  [PASS] Cargo.lock"
else
  echo "  [FAIL] Cargo.lock -- y-agent version is not $NEW_VERSION"
  ERRORS=$((ERRORS + 1))
fi

echo ""
if [ "$ERRORS" -gt 0 ]; then
  die "Verification failed with $ERRORS error(s). Please check the files manually."
fi

echo "Done! Version bumped to $NEW_VERSION across all 6 files."
echo ""
echo "Next steps:"
echo "  git add -u"
echo "  git commit -m \"chore: bump version to $NEW_VERSION\""
echo "  git tag v$NEW_VERSION"
echo "  git push origin v$NEW_VERSION"
