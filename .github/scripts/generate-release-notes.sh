#!/usr/bin/env bash
# =============================================================================
# generate-release-notes.sh -- Generate structured release notes with download
# badges, categorized changelog, and verification instructions.
#
# Usage (from GitHub Actions):
#   ./.github/scripts/generate-release-notes.sh
#
# Usage (local):
#   VERSION=0.1.3 GITHUB_REPOSITORY=gorgiaxx/y-agent \
#     ./.github/scripts/generate-release-notes.sh
#
# Environment Variables:
#   GITHUB_REF_NAME     Tag name (e.g. v0.1.3) -- set by GitHub Actions
#   GITHUB_REPOSITORY   owner/repo             -- set by GitHub Actions
#   GITHUB_SHA          Full commit SHA         -- set by GitHub Actions
#   VERSION             Override version string (optional)
#   DIST_DIR            Directory containing built artifacts (default: dist)
#   OUTPUT_FILE         Output file path (default: RELEASE_NOTES.md)
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DIST_DIR="${DIST_DIR:-dist}"
OUTPUT_FILE="${OUTPUT_FILE:-RELEASE_NOTES.md}"

# -- Version resolution --------------------------------------------------------
# Priority: VERSION env > valid tag > short SHA > Cargo.toml
resolve_version() {
  if [[ -n "${VERSION:-}" ]]; then
    echo "$VERSION"
    return
  fi

  local ref_name="${GITHUB_REF_NAME:-}"
  if [[ "$ref_name" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    echo "${ref_name#v}"
    return
  fi

  local sha="${GITHUB_SHA:-}"
  if [[ -n "$sha" ]]; then
    echo "${sha:0:7}"
    return
  fi

  # Fallback: read from Cargo.toml
  grep -m1 'version' "$REPO_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

VERSION="$(resolve_version)"
REPO="${GITHUB_REPOSITORY:-gorgiaxx/y-agent}"

# Determine tag for download URLs
REF_NAME="${GITHUB_REF_NAME:-}"
if [[ "$REF_NAME" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  TAG="$REF_NAME"
else
  TAG="v${VERSION}"
fi

BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

echo "Generating release notes..."
echo "  Version:    ${VERSION}"
echo "  Tag:        ${TAG}"
echo "  Repository: ${REPO}"
echo "  Output:     ${OUTPUT_FILE}"

# -- Generate categorized changelog --------------------------------------------
generate_changelog() {
  local prev_tag
  prev_tag="$(git describe --tags --abbrev=0 HEAD^ 2>/dev/null || echo "")"

  if [[ -z "$prev_tag" ]]; then
    echo "Initial release."
    return
  fi

  local range="${prev_tag}..HEAD"
  local has_content=false

  # Categories: features, fixes, documentation, refactoring, other
  local -a feats=() fixes=() docs=() refactors=() chores=() others=()

  while IFS= read -r line; do
    local subject hash
    hash="$(echo "$line" | awk '{print $1}')"
    subject="$(echo "$line" | cut -d' ' -f2-)"

    case "$subject" in
      feat:*|feat\(*) feats+=("- ${subject} (\`${hash}\`)") ;;
      fix:*|fix\(*)   fixes+=("- ${subject} (\`${hash}\`)") ;;
      docs:*|doc:*)   docs+=("- ${subject} (\`${hash}\`)") ;;
      refactor:*)     refactors+=("- ${subject} (\`${hash}\`)") ;;
      chore:*|ci:*|build:*|style:*|test:*) chores+=("- ${subject} (\`${hash}\`)") ;;
      *)              others+=("- ${subject} (\`${hash}\`)") ;;
    esac
  done < <(git log --pretty=format:"%h %s" "$range" 2>/dev/null)

  if [[ ${#feats[@]} -gt 0 ]]; then
    echo "### New Features"
    echo ""
    printf '%s\n' "${feats[@]}"
    echo ""
    has_content=true
  fi

  if [[ ${#fixes[@]} -gt 0 ]]; then
    echo "### Bug Fixes"
    echo ""
    printf '%s\n' "${fixes[@]}"
    echo ""
    has_content=true
  fi

  if [[ ${#refactors[@]} -gt 0 ]]; then
    echo "### Refactoring"
    echo ""
    printf '%s\n' "${refactors[@]}"
    echo ""
    has_content=true
  fi

  if [[ ${#docs[@]} -gt 0 ]]; then
    echo "### Documentation"
    echo ""
    printf '%s\n' "${docs[@]}"
    echo ""
    has_content=true
  fi

  if [[ ${#others[@]} -gt 0 ]]; then
    echo "### Other Changes"
    echo ""
    printf '%s\n' "${others[@]}"
    echo ""
    has_content=true
  fi

  if [[ ${#chores[@]} -gt 0 ]]; then
    echo "<details>"
    echo "<summary>Maintenance (${#chores[@]} changes)</summary>"
    echo ""
    printf '%s\n' "${chores[@]}"
    echo ""
    echo "</details>"
    echo ""
    has_content=true
  fi

  if [[ "$has_content" != true ]]; then
    echo "No notable changes since ${prev_tag}."
  fi
}

# -- Badge helper --------------------------------------------------------------
# badge <label> <message> <color> <logo> <download_url>
badge() {
  local label="$1" message="$2" color="$3" logo="$4" url="$5"
  local encoded_label encoded_message
  encoded_label="$(echo "$label" | sed 's/ /_/g; s/-/--/g')"
  encoded_message="$(echo "$message" | sed 's/ /_/g; s/-/--/g')"
  echo "[![${label} ${message}](https://img.shields.io/badge/${encoded_label}-${encoded_message}-${color}?style=flat-square&logo=${logo})](${url})"
}

# -- Artifact detection --------------------------------------------------------
# Check if an artifact or glob pattern matches any file in DIST_DIR
has_artifact() {
  compgen -G "${DIST_DIR}/$1" > /dev/null 2>&1
}

get_artifact_name() {
  local pattern="$1"
  basename "$(compgen -G "${DIST_DIR}/${pattern}" | head -1)" 2>/dev/null || echo ""
}

# -- Build release notes -------------------------------------------------------
{
  # Header
  cat <<EOF
## What's Changed

EOF

  generate_changelog

  cat <<EOF
---

## Downloads

> Verify integrity with: \`sha256sum -c SHA256SUMS.txt\`

EOF

  # CLI downloads table
  cat <<EOF
### CLI (command-line)

| Platform | Architecture | Download |
| :------- | :----------- | :------- |
EOF

  # macOS CLI
  if has_artifact "y-agent-cli-*-darwin-arm64.tar.gz"; then
    fname="$(get_artifact_name "y-agent-cli-*-darwin-arm64.tar.gz")"
    echo "| **macOS** | Apple Silicon (arm64) | $(badge "tar.gz" "arm64" "000000" "apple" "${BASE_URL}/${fname}") |"
  fi
  if has_artifact "y-agent-cli-*-darwin-amd64.tar.gz"; then
    fname="$(get_artifact_name "y-agent-cli-*-darwin-amd64.tar.gz")"
    echo "| **macOS** | Intel (x64) | $(badge "tar.gz" "x64" "000000" "apple" "${BASE_URL}/${fname}") |"
  fi

  # Linux CLI
  if has_artifact "y-agent-cli-*-linux-amd64.tar.gz"; then
    fname="$(get_artifact_name "y-agent-cli-*-linux-amd64.tar.gz")"
    echo "| **Linux** | x64 | $(badge "tar.gz" "x64" "FCC624" "linux" "${BASE_URL}/${fname}") |"
  fi
  if has_artifact "y-agent-cli-*-linux-arm64.tar.gz"; then
    fname="$(get_artifact_name "y-agent-cli-*-linux-arm64.tar.gz")"
    echo "| **Linux** | arm64 | $(badge "tar.gz" "arm64" "FCC624" "linux" "${BASE_URL}/${fname}") |"
  fi

  # Windows CLI
  if has_artifact "y-agent-cli-*-windows-amd64.zip"; then
    fname="$(get_artifact_name "y-agent-cli-*-windows-amd64.zip")"
    echo "| **Windows** | x64 | $(badge "zip" "x64" "0078D6" "windows" "${BASE_URL}/${fname}") |"
  fi
  if has_artifact "y-agent-cli-*-windows-arm64.zip"; then
    fname="$(get_artifact_name "y-agent-cli-*-windows-arm64.zip")"
    echo "| **Windows** | arm64 | $(badge "zip" "arm64" "0078D6" "windows" "${BASE_URL}/${fname}") |"
  fi

  echo ""

  # GUI downloads table
  cat <<EOF
### GUI (desktop app)

| Platform | Format | Download |
| :------- | :----- | :------- |
EOF

  # macOS GUI
  for dmg in ${DIST_DIR}/*.dmg; do
    [[ -f "$dmg" ]] || continue
    fname="$(basename "$dmg")"
    if echo "$fname" | grep -qi "arm64\|aarch64"; then
      echo "| **macOS** | DMG (Apple Silicon) | $(badge "DMG" "Apple_Silicon" "000000" "apple" "${BASE_URL}/${fname}") |"
    elif echo "$fname" | grep -qi "x64\|amd64\|x86_64"; then
      echo "| **macOS** | DMG (Intel x64) | $(badge "DMG" "Intel_x64" "000000" "apple" "${BASE_URL}/${fname}") |"
    else
      echo "| **macOS** | DMG | $(badge "DMG" "Universal" "000000" "apple" "${BASE_URL}/${fname}") |"
    fi
  done

  # Linux GUI
  for deb in ${DIST_DIR}/*.deb; do
    [[ -f "$deb" ]] || continue
    fname="$(basename "$deb")"
    if echo "$fname" | grep -qi "arm64\|aarch64"; then
      echo "| **Linux** | DEB (arm64) | $(badge "DEB" "arm64" "A80030" "debian" "${BASE_URL}/${fname}") |"
    else
      echo "| **Linux** | DEB (x64) | $(badge "DEB" "x64" "A80030" "debian" "${BASE_URL}/${fname}") |"
    fi
  done

  for appimage in ${DIST_DIR}/*.AppImage; do
    [[ -f "$appimage" ]] || continue
    fname="$(basename "$appimage")"
    if echo "$fname" | grep -qi "arm64\|aarch64"; then
      echo "| **Linux** | AppImage (arm64) | $(badge "AppImage" "arm64" "FCC624" "linux" "${BASE_URL}/${fname}") |"
    else
      echo "| **Linux** | AppImage (x64) | $(badge "AppImage" "x64" "FCC624" "linux" "${BASE_URL}/${fname}") |"
    fi
  done

  for rpm in ${DIST_DIR}/*.rpm; do
    [[ -f "$rpm" ]] || continue
    fname="$(basename "$rpm")"
    if echo "$fname" | grep -qi "arm64\|aarch64"; then
      echo "| **Linux** | RPM (arm64) | $(badge "RPM" "arm64" "CC0000" "redhat" "${BASE_URL}/${fname}") |"
    else
      echo "| **Linux** | RPM (x64) | $(badge "RPM" "x64" "CC0000" "redhat" "${BASE_URL}/${fname}") |"
    fi
  done

  for pacman_pkg in ${DIST_DIR}/*.pkg.tar.zst; do
    [[ -f "$pacman_pkg" ]] || continue
    fname="$(basename "$pacman_pkg")"
    echo "| **Linux** | Pacman (x64) | $(badge "Pacman" "x64" "1793D1" "archlinux" "${BASE_URL}/${fname}") |"
  done

  # Windows GUI
  for msi in ${DIST_DIR}/*.msi; do
    [[ -f "$msi" ]] || continue
    fname="$(basename "$msi")"
    echo "| **Windows** | MSI | $(badge "MSI" "x64" "0078D6" "windows" "${BASE_URL}/${fname}") |"
  done

  for nsis_exe in ${DIST_DIR}/*.exe; do
    [[ -f "$nsis_exe" ]] || continue
    fname="$(basename "$nsis_exe")"
    echo "| **Windows** | Setup | $(badge "Setup" "x64" "0078D6" "windows" "${BASE_URL}/${fname}") |"
  done

  echo ""

  # Installation instructions
  cat <<EOF
---

### Quick Install (CLI)

**macOS / Linux:**
\`\`\`bash
# Using the install script
curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/native-install.sh | bash

# Or manually
tar xzf y-agent-cli-${VERSION}-<platform>.tar.gz
cd y-agent-cli-${VERSION}-<platform>
./y-agent --help
\`\`\`

**Windows (PowerShell):**
\`\`\`powershell
Expand-Archive y-agent-cli-${VERSION}-windows-amd64.zip -DestinationPath .
.\y-agent-cli-${VERSION}-windows-amd64\y-agent.exe --help
\`\`\`

### Verification

\`\`\`bash
# Download SHA256SUMS.txt from the release assets, then:
sha256sum -c SHA256SUMS.txt
\`\`\`
EOF

} > "$OUTPUT_FILE"

echo ""
echo "Release notes written to: ${OUTPUT_FILE}"
echo "$(wc -l < "$OUTPUT_FILE") lines generated."
