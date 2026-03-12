#!/usr/bin/env bash
# ==============================================================================
# y-agent Deployment Script
#
# Usage:
#   ./scripts/deploy.sh [VERSION]
#
# Arguments:
#   VERSION   Docker image tag to deploy (default: latest)
#
# Environment:
#   DEPLOY_DIR    Path to deployment directory (default: /opt/y-agent)
#   REGISTRY      Container registry (default: ghcr.io/gorgias/y-agent)
# ==============================================================================
set -euo pipefail

VERSION="${1:-latest}"
DEPLOY_DIR="${DEPLOY_DIR:-/opt/y-agent}"
REGISTRY="${REGISTRY:-ghcr.io/gorgias/y-agent}"
IMAGE="${REGISTRY}:${VERSION}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()   { echo -e "${GREEN}[DEPLOY]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# ── Pre-checks ───────────────────────────────────────────────────────────────

if ! command -v docker &> /dev/null; then
    error "docker is not installed"
    exit 1
fi

if ! docker compose version &> /dev/null; then
    error "docker compose is not available"
    exit 1
fi

if [ ! -d "${DEPLOY_DIR}" ]; then
    error "Deploy directory does not exist: ${DEPLOY_DIR}"
    exit 1
fi

cd "${DEPLOY_DIR}"

# ── Save rollback info ───────────────────────────────────────────────────────

PREVIOUS_IMAGE=""
if docker compose ps --format json 2>/dev/null | jq -r '.[0].Image // empty' > /dev/null 2>&1; then
    PREVIOUS_IMAGE=$(docker compose ps --format json | jq -r '.[0].Image // empty')
fi
echo "${PREVIOUS_IMAGE}" > .previous-image
log "Previous image: ${PREVIOUS_IMAGE:-none}"

# ── Pull new image ───────────────────────────────────────────────────────────

log "Pulling image: ${IMAGE}"
if ! docker pull "${IMAGE}"; then
    error "Failed to pull image: ${IMAGE}"
    exit 1
fi

# ── Update .env ──────────────────────────────────────────────────────────────

if grep -q "^Y_AGENT_IMAGE=" .env 2>/dev/null; then
    sed -i.bak "s|^Y_AGENT_IMAGE=.*|Y_AGENT_IMAGE=${IMAGE}|" .env
else
    echo "Y_AGENT_IMAGE=${IMAGE}" >> .env
fi
log "Updated .env with image: ${IMAGE}"

# ── Deploy ───────────────────────────────────────────────────────────────────

log "Starting deployment..."
docker compose up -d --remove-orphans

# ── Health check ─────────────────────────────────────────────────────────────

log "Running health checks..."
if ! ./scripts/health-check.sh; then
    error "Health check failed! Initiating rollback..."

    if [ -n "${PREVIOUS_IMAGE}" ]; then
        warn "Rolling back to: ${PREVIOUS_IMAGE}"
        sed -i.bak "s|^Y_AGENT_IMAGE=.*|Y_AGENT_IMAGE=${PREVIOUS_IMAGE}|" .env
        docker compose up -d --remove-orphans

        sleep 5
        if ./scripts/health-check.sh; then
            log "Rollback successful"
        else
            error "Rollback also failed! Manual intervention required."
        fi
    else
        error "No previous image available for rollback. Manual intervention required."
    fi
    exit 1
fi

# ── Cleanup ──────────────────────────────────────────────────────────────────

log "Cleaning up old images..."
docker image prune -f --filter "until=168h" 2>/dev/null || true

log "Deployment successful! Image: ${IMAGE}"
