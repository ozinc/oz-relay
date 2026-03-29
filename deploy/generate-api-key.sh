#!/usr/bin/env bash
# Generate a bsk_ API key for a developer.
# Usage: ./generate-api-key.sh <developer_id> <tier> [secret]
#
# FIX #1: Rewrote to avoid shell injection. Uses Python with proper
# argument passing instead of string interpolation.
#
# Example:
#   ./generate-api-key.sh dev_alice community
#   ./generate-api-key.sh dev_bob professional "your-jwt-secret"

set -euo pipefail

DEV_ID="${1:?Usage: $0 <developer_id> <tier> [jwt_secret]}"
TIER="${2:?Usage: $0 <developer_id> <tier> [jwt_secret]}"
SECRET="${3:-${RELAY_JWT_SECRET:?Set RELAY_JWT_SECRET or pass as 3rd argument}}"

# Validate tier
case "$TIER" in
    community|professional|enterprise) ;;
    *) echo "error: tier must be community, professional, or enterprise"; exit 1 ;;
esac

# Validate developer_id: alphanumeric, underscores, hyphens only
if ! [[ "$DEV_ID" =~ ^[a-zA-Z0-9_-]+$ ]]; then
    echo "error: developer_id must contain only alphanumeric characters, underscores, and hyphens"
    exit 1
fi

# Generate JWT using python with safe argument passing (no string interpolation)
if ! python3 -c "import jwt" 2>/dev/null; then
    echo "installing PyJWT..."
    pip3 install -q PyJWT
fi

TOKEN=$(python3 - "$DEV_ID" "$TIER" "$SECRET" <<'PYEOF'
import sys, time, jwt

dev_id = sys.argv[1]
tier = sys.argv[2]
secret = sys.argv[3]
expiry = int(time.time()) + 365 * 24 * 3600

token = jwt.encode(
    {"sub": dev_id, "tier": tier, "exp": expiry},
    secret,
    algorithm="HS256"
)
print(token)
PYEOF
)

EXPIRY=$(python3 -c "import time; print(int(time.time()) + 365 * 24 * 3600)")

echo ""
echo "Developer: $DEV_ID"
echo "Tier:      $TIER"
echo "Expires:   $(date -d @$EXPIRY 2>/dev/null || date -r $EXPIRY) (1 year)"
echo ""
echo "API Key:"
echo "  bsk_$TOKEN"
echo ""
echo "Usage:"
echo "  export RELAY_API_KEY=bsk_$TOKEN"
echo "  arcflow-relay discover --server https://relay.oz.global"
