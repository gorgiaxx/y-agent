#!/usr/bin/env bash
# ==============================================================================
# y-agent Health Check Script
#
# Checks the health of all services in the y-agent stack.
# Exits 0 if all healthy, 1 otherwise.
#
# Usage:
#   ./scripts/health-check.sh [--wait SECONDS]
#
# Options:
#   --wait SECONDS   Maximum time to wait for services (default: 60)
# ==============================================================================
set -euo pipefail

MAX_WAIT=60
AGENT_URL="${AGENT_URL:-http://localhost:8080}"
PG_HOST="${PG_HOST:-localhost}"
PG_PORT="${PG_PORT:-5432}"
QDRANT_URL="${QDRANT_URL:-http://localhost:6333}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --wait) MAX_WAIT="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "  ${GREEN}✓${NC} $*"; }
fail() { echo -e "  ${RED}✗${NC} $*"; }
wait_msg() { echo -e "  ${YELLOW}⏳${NC} $*"; }

HEALTHY=true

echo "Running health checks (timeout: ${MAX_WAIT}s)..."
echo ""

# ── y-agent ──────────────────────────────────────────────────────────────────

echo "y-agent:"
ELAPSED=0
until curl -sf "${AGENT_URL}/health" > /dev/null 2>&1; do
    ELAPSED=$((ELAPSED + 2))
    if [ $ELAPSED -ge $MAX_WAIT ]; then
        fail "HTTP health endpoint unreachable at ${AGENT_URL}/health"
        HEALTHY=false
        break
    fi
    wait_msg "Waiting for y-agent... (${ELAPSED}s/${MAX_WAIT}s)"
    sleep 2
done

if [ "$HEALTHY" = true ] || curl -sf "${AGENT_URL}/health" > /dev/null 2>&1; then
    pass "HTTP health endpoint OK"
fi

# ── PostgreSQL ───────────────────────────────────────────────────────────────

echo ""
echo "PostgreSQL:"
if command -v pg_isready &> /dev/null; then
    if pg_isready -h "${PG_HOST}" -p "${PG_PORT}" > /dev/null 2>&1; then
        pass "Connection OK (${PG_HOST}:${PG_PORT})"
    else
        fail "Connection failed (${PG_HOST}:${PG_PORT})"
        HEALTHY=false
    fi
elif docker compose exec -T postgres pg_isready > /dev/null 2>&1; then
    pass "Connection OK (via docker compose)"
else
    fail "Connection check failed"
    HEALTHY=false
fi

# ── Qdrant ───────────────────────────────────────────────────────────────────

echo ""
echo "Qdrant:"
if curl -sf "${QDRANT_URL}/healthz" > /dev/null 2>&1; then
    pass "HTTP health OK (${QDRANT_URL})"
else
    fail "HTTP health failed (${QDRANT_URL})"
    HEALTHY=false
fi

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
if [ "$HEALTHY" = true ]; then
    echo -e "${GREEN}All health checks passed!${NC}"
    exit 0
else
    echo -e "${RED}Some health checks failed!${NC}"
    exit 1
fi
