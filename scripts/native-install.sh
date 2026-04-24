#!/usr/bin/env bash
# ==============================================================================
# y-agent Native Installation Script
#
# Installs y-agent directly on the host machine (no Docker required).
#
# Usage:
#   ./scripts/native-install.sh [OPTIONS]
#
# Options:
#   --prefix DIR     Installation prefix (default: /usr/local)
#   --data-dir DIR   Data directory (default: ~/.local/share/y-agent)
#   --release        Build in release mode (default)
#   --debug          Build in debug mode
#   --skip-build     Skip the build step (use existing binary)
#   --help           Show this help
# ==============================================================================
set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────────────────────

PREFIX="/usr/local"
DATA_DIR="${HOME}/.local/share/y-agent"
CONFIG_DIR="${HOME}/.config/y-agent"
BUILD_MODE="release"
SKIP_BUILD=false
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()   { echo -e "${GREEN}[INSTALL]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}    $*"; }
info()  { echo -e "${BLUE}[INFO]${NC}    $*"; }
error() { echo -e "${RED}[ERROR]${NC}   $*" >&2; }

# ── Parse Arguments ──────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case $1 in
        --prefix)     PREFIX="$2"; shift 2 ;;
        --data-dir)   DATA_DIR="$2"; shift 2 ;;
        --release)    BUILD_MODE="release"; shift ;;
        --debug)      BUILD_MODE="debug"; shift ;;
        --skip-build) SKIP_BUILD=true; shift ;;
        --help)
            head -20 "$0" | grep '^#' | sed 's/^# \?//'
            exit 0
            ;;
        *) error "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Pre-checks ───────────────────────────────────────────────────────────────

log "y-agent Native Installation"
echo ""

# Check Rust toolchain
if ! command -v cargo &> /dev/null; then
    error "Rust/Cargo is not installed."
    info "Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

RUST_VERSION=$(rustc --version | awk '{print $2}')
info "Rust version: ${RUST_VERSION}"

# Check SQLite
if ! command -v sqlite3 &> /dev/null; then
    warn "sqlite3 CLI not found (optional, but recommended for debugging)"
fi

# ── Build ────────────────────────────────────────────────────────────────────

if [ "${SKIP_BUILD}" = false ]; then
    log "Building y-agent (${BUILD_MODE} mode)..."
    cd "${PROJECT_DIR}"

    if [ "${BUILD_MODE}" = "release" ]; then
        cargo build --release --bin y-agent
        BINARY_PATH="${PROJECT_DIR}/target/release/y-agent"
    else
        cargo build --bin y-agent
        BINARY_PATH="${PROJECT_DIR}/target/debug/y-agent"
    fi

    log "Build complete: ${BINARY_PATH}"
else
    if [ "${BUILD_MODE}" = "release" ]; then
        BINARY_PATH="${PROJECT_DIR}/target/release/y-agent"
    else
        BINARY_PATH="${PROJECT_DIR}/target/debug/y-agent"
    fi

    if [ ! -f "${BINARY_PATH}" ]; then
        error "Binary not found: ${BINARY_PATH}"
        info "Run without --skip-build or build manually first."
        exit 1
    fi
fi

# ── Install Binary ──────────────────────────────────────────────────────────

BIN_DIR="${PREFIX}/bin"
log "Installing binary to ${BIN_DIR}/y-agent..."

if [ -w "${BIN_DIR}" ]; then
    cp "${BINARY_PATH}" "${BIN_DIR}/y-agent"
else
    warn "Need sudo to install to ${BIN_DIR}"
    sudo cp "${BINARY_PATH}" "${BIN_DIR}/y-agent"
fi

chmod +x "${BIN_DIR}/y-agent"

# ── Create Data Directories ─────────────────────────────────────────────────

log "Creating directories..."
mkdir -p "${DATA_DIR}"
mkdir -p "${DATA_DIR}/transcripts"
mkdir -p "${CONFIG_DIR}"

# ── Copy Configuration ──────────────────────────────────────────────────────

if [ ! -f "${CONFIG_DIR}/y-agent.toml" ]; then
    log "Creating config files at ${CONFIG_DIR}/..."

    # Copy all example config files, renaming .example.toml → .toml
    for example_file in "${PROJECT_DIR}"/config/*.example.toml; do
        basename=$(basename "${example_file}" .example.toml)
        target="${CONFIG_DIR}/${basename}.toml"
        cp "${example_file}" "${target}"
    done

    # Update paths in storage.toml to use the actual data directory
    STORAGE_FILE="${CONFIG_DIR}/storage.toml"
    if [ -f "${STORAGE_FILE}" ]; then
        if [[ "$OSTYPE" == "darwin"* ]]; then
            sed -i '' "s|db_path = \"data/y-agent.db\"|db_path = \"${DATA_DIR}/y-agent.db\"|" "${STORAGE_FILE}"
            sed -i '' "s|transcript_dir = \"data/transcripts\"|transcript_dir = \"${DATA_DIR}/transcripts\"|" "${STORAGE_FILE}"
        else
            sed -i "s|db_path = \"data/y-agent.db\"|db_path = \"${DATA_DIR}/y-agent.db\"|" "${STORAGE_FILE}"
            sed -i "s|transcript_dir = \"data/transcripts\"|transcript_dir = \"${DATA_DIR}/transcripts\"|" "${STORAGE_FILE}"
        fi
    fi

    warn "Edit files in ${CONFIG_DIR}/ to configure your LLM provider API key."
else
    info "Config already exists at ${CONFIG_DIR}/, skipping."
fi

# -- Copy Skills --------------------------------------------------------------

SKILLS_SRC="${PROJECT_DIR}/skills"
SKILLS_DST="${DATA_DIR}/skills"
if [ -d "${SKILLS_SRC}" ]; then
    log "Copying skills to ${SKILLS_DST}/..."
    mkdir -p "${SKILLS_DST}"
    for skill_dir in "${SKILLS_SRC}"/*/; do
        skill_name="$(basename "${skill_dir}")"
        if [ ! -d "${SKILLS_DST}/${skill_name}" ]; then
            cp -r "${skill_dir}" "${SKILLS_DST}/${skill_name}"
            info "  Installed skill: ${skill_name}"
        else
            info "  Skill already exists: ${skill_name} (skipped)"
        fi
    done
fi

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  y-agent installed successfully!${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════════${NC}"
echo ""
echo "  Binary:    ${BIN_DIR}/y-agent"
echo "  Config:    ${CONFIG_FILE}"
echo "  Data:      ${DATA_DIR}"
echo ""
echo "  Next steps:"
echo "    1. Set your API key:  export OPENAI_API_KEY=\"sk-...\""
echo "    2. Edit config:       \$EDITOR ${CONFIG_FILE}"
echo "    3. Start chatting:    y-agent chat"
echo ""

# Verify installation
if command -v y-agent &> /dev/null; then
    INSTALLED_VERSION=$(y-agent --version 2>/dev/null || echo "unknown")
    info "Installed version: ${INSTALLED_VERSION}"
else
    warn "y-agent not found in PATH. Add ${BIN_DIR} to your PATH."
fi
