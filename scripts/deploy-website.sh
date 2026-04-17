#!/usr/bin/env bash
# ==============================================================================
# deploy-website.sh -- Build and deploy the y-agent website to Cloudflare Pages
#
# Usage:
#   ./scripts/deploy-website.sh [OPTIONS]
#
# Options:
#   --preview       Deploy to preview branch (default: production)
#   --no-build      Skip build step (deploy existing dist only)
#   -h, --help      Show this help message
#
# Environment:
#   CF_PROJECT      Cloudflare Pages project name (default: y-agent)
#   CF_BRANCH       Production branch name (default: main)
#
# Prerequisites:
#   - pnpm
#   - wrangler (npm install -g wrangler)
#   - Cloudflare account with Pages project configured
#
# Examples:
#   ./scripts/deploy-website.sh                  # Build + deploy to production
#   ./scripts/deploy-website.sh --preview        # Build + deploy to preview
#   ./scripts/deploy-website.sh --no-build       # Deploy existing dist only
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WEBSITE_DIR="$PROJECT_ROOT/website"

# -- Defaults ------------------------------------------------------------------
CF_PROJECT="${CF_PROJECT:-y-agent}"
CF_BRANCH="${CF_BRANCH:-main}"
DEPLOY_MODE="production"
SKIP_BUILD=false

# -- Parse arguments -----------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --preview)
      DEPLOY_MODE="preview"; shift ;;
    --no-build)
      SKIP_BUILD=true; shift ;;
    -h|--help)
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

# -- Colors --------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${GREEN}[WEBSITE]${NC} $*"; }
info() { echo -e "${CYAN}[INFO]${NC}    $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC}   $*"; }
err()  { echo -e "${RED}[ERROR]${NC}  $*" >&2; }

# -- Pre-checks ----------------------------------------------------------------

if ! command -v pnpm &> /dev/null; then
  err "pnpm is not installed. Install it with: npm install -g pnpm"
  exit 1
fi

if ! command -v wrangler &> /dev/null; then
  err "wrangler is not installed. Install it with: npm install -g wrangler"
  exit 1
fi

if [[ ! -d "$WEBSITE_DIR" ]]; then
  err "Website directory not found: $WEBSITE_DIR"
  exit 1
fi

DIST_DIR="$WEBSITE_DIR/docs/.vitepress/dist"

# -- Build ---------------------------------------------------------------------

if [[ "$SKIP_BUILD" == false ]]; then
  log "Installing dependencies..."
  (cd "$WEBSITE_DIR" && pnpm install --frozen-lockfile 2>/dev/null || pnpm install)

  log "Building VitePress site..."
  (cd "$WEBSITE_DIR" && pnpm build)

  if [[ ! -d "$DIST_DIR" ]]; then
    err "Build output not found: $DIST_DIR"
    exit 1
  fi
else
  if [[ ! -d "$DIST_DIR" ]]; then
    err "No existing build found at $DIST_DIR. Run without --no-build first."
    exit 1
  fi
  log "Skipping build (using existing dist)"
fi

# -- Deploy --------------------------------------------------------------------

WRANGLER_ARGS=(pages deploy "$DIST_DIR" --project-name "$CF_PROJECT")

if [[ "$DEPLOY_MODE" == "production" ]]; then
  WRANGLER_ARGS+=(--branch "$CF_BRANCH")
  log "Deploying to PRODUCTION ($CF_PROJECT.pages.dev)..."
else
  log "Deploying to PREVIEW..."
fi

DEPLOY_OUTPUT=$(cd "$WEBSITE_DIR" && wrangler "${WRANGLER_ARGS[@]}" 2>&1)
echo "$DEPLOY_OUTPUT"

# -- Extract URL ---------------------------------------------------------------

DEPLOY_URL=$(echo "$DEPLOY_OUTPUT" | grep -oE 'https://[a-z0-9-]+\.'$CF_PROJECT'\.pages\.dev' | head -1)
ALIAS_URL=$(echo "$DEPLOY_OUTPUT" | grep -oE 'https://head\.'$CF_PROJECT'\.pages\.dev' | head -1)

echo ""
log "====================================="
if [[ "$DEPLOY_MODE" == "production" ]]; then
  log "Production deployment complete"
  info "URL: https://$CF_PROJECT.pages.dev"
else
  log "Preview deployment complete"
  info "URL: ${DEPLOY_URL:-unknown}"
fi
[[ -n "$ALIAS_URL" ]] && info "Alias: $ALIAS_URL"
log "====================================="
