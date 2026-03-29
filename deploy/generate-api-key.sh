#!/usr/bin/env bash
# Generate a bsk_ API key for a developer.
# Usage: ./generate-api-key.sh <developer_id> <tier> [status] [secret]
#
# Status controls entitlement:
#   pending   — default, can query tasks but cannot submit intents
#   active    — approved, can submit intents (burns server-side tokens)
#   suspended — revoked, rejected at auth layer
#
# Example:
#   ./generate-api-key.sh dev_alice community                    # pending key
#   ./generate-api-key.sh dev_alice community active             # approved key
#   ./generate-api-key.sh dev_bob professional active "secret"   # with explicit secret

set -euo pipefail

DEV_ID="${1:?Usage: $0 <developer_id> <tier> [status] [jwt_secret]}"
TIER="${2:?Usage: $0 <developer_id> <tier> [status] [jwt_secret]}"
STATUS="${3:-pending}"
SECRET="${4:-${RELAY_JWT_SECRET:?Set RELAY_JWT_SECRET or pass as 4th argument}}"

# Validate tier
case "$TIER" in
    community|professional|enterprise) ;;
    *) echo "error: tier must be community, professional, or enterprise"; exit 1 ;;
esac

# Validate status
case "$STATUS" in
    active|pending|suspended) ;;
    *) echo "error: status must be active, pending, or suspended"; exit 1 ;;
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

TOKEN=$(python3 - "$DEV_ID" "$TIER" "$STATUS" "$SECRET" <<'PYEOF'
import sys, time, jwt

dev_id = sys.argv[1]
tier = sys.argv[2]
status = sys.argv[3]
secret = sys.argv[4]
expiry = int(time.time()) + 365 * 24 * 3600

token = jwt.encode(
    {"sub": dev_id, "tier": tier, "status": status, "exp": expiry},
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
echo "Status:    $STATUS"
echo "Expires:   $(date -d @$EXPIRY 2>/dev/null || date -r $EXPIRY) (1 year)"
echo ""
if [ "$STATUS" = "pending" ]; then
    echo "NOTE: This key is PENDING. The developer can query task status"
    echo "      but cannot submit intents until the key is re-issued as 'active'."
    echo ""
fi
echo "API Key:"
echo "  bsk_$TOKEN"
echo ""
echo "Usage:"
echo "  export RELAY_API_KEY=bsk_$TOKEN"
echo "  arcflow-relay discover --server https://relay.oz.global"
