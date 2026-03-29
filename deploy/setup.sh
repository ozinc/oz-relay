#!/usr/bin/env bash
# OZ Relay — Ubuntu Server Setup
# Run as root or with sudo on a fresh Ubuntu 22.04+ server.
#
# Prerequisites: Ubuntu 22.04+, systemd, git, curl
# The server should already have Rust toolchain and NVIDIA drivers.

set -euo pipefail

RELAY_USER="oz-relay"
RELAY_HOME="/opt/oz-relay"
ARCFLOW_REPO="/opt/arcflow"
PROMOTIONS_DIR="/opt/promotions"
# FIX #10: Pin nsjail to a known release tag
NSJAIL_VERSION="3.4"

echo "=== OZ Relay Setup ==="

# 1. Create service user
if ! id "$RELAY_USER" &>/dev/null; then
    useradd --system --shell /usr/sbin/nologin --home-dir "$RELAY_HOME" "$RELAY_USER"
    echo "created user: $RELAY_USER"
fi

# 2. Create directory structure
mkdir -p "$RELAY_HOME"/{bin,config,keys,logs}
mkdir -p "$ARCFLOW_REPO"/.relay-worktrees
mkdir -p "$PROMOTIONS_DIR"

# 3. Install nsjail for sandbox isolation
# FIX #10: Pin to release tag instead of HEAD
if ! command -v nsjail &>/dev/null; then
    echo "installing nsjail v${NSJAIL_VERSION}..."
    apt-get update -qq
    apt-get install -y -qq autoconf bison flex gcc g++ git libprotobuf-dev \
        libnl-route-3-dev libtool make pkg-config protobuf-compiler
    NSJAIL_TMP=$(mktemp -d)
    git clone --branch "${NSJAIL_VERSION}" --depth 1 \
        https://github.com/google/nsjail.git "$NSJAIL_TMP"
    cd "$NSJAIL_TMP" && make -j"$(nproc)" && cp nsjail /usr/local/bin/
    rm -rf "$NSJAIL_TMP"
    echo "nsjail v${NSJAIL_VERSION} installed at /usr/local/bin/nsjail"
fi

# 4. Install Caddy for TLS termination
if ! command -v caddy &>/dev/null; then
    echo "installing caddy..."
    apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | \
        gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | \
        tee /etc/apt/sources.list.d/caddy-stable.list
    apt-get update -qq && apt-get install -y -qq caddy
    echo "caddy installed"
fi

# 5. Install Claude Code
if ! command -v claude &>/dev/null; then
    echo "installing claude code..."
    npm install -g @anthropic-ai/claude-code
    echo "claude code installed"
fi

# 6. Generate Ed25519 signing keypair (if not exists)
if [ ! -f "$RELAY_HOME/keys/signing.pkcs8" ]; then
    echo "generating Ed25519 signing keypair..."
    openssl genpkey -algorithm Ed25519 -out "$RELAY_HOME/keys/signing.pkcs8"
    openssl pkey -in "$RELAY_HOME/keys/signing.pkcs8" -pubout \
        -out "$RELAY_HOME/keys/signing.pub"
    echo "keypair generated. publish signing.pub to the SDK repo."
fi

# 7. Clone ArcFlow source (bare repo)
if [ ! -d "$ARCFLOW_REPO/repo.git" ]; then
    echo "you need to run: git clone --bare git@github.com:ozinc/arcflow-core.git $ARCFLOW_REPO/repo.git"
    echo "(requires SSH key with repo access)"
fi

# 8. Set ownership and permissions
# FIX #12, #14: Restrictive permissions on sensitive directories
chown -R "$RELAY_USER":"$RELAY_USER" "$RELAY_HOME" "$ARCFLOW_REPO" "$PROMOTIONS_DIR"
chmod 700 "$RELAY_HOME/keys"
chmod 600 "$RELAY_HOME/keys"/*  2>/dev/null || true
chmod 700 "$RELAY_HOME/config"

# 9. Install config template if not already configured
if [ ! -f "$RELAY_HOME/config/config.env" ]; then
    cp "$(dirname "$0")/config.env.example" "$RELAY_HOME/config/config.env"
    chown "$RELAY_USER":"$RELAY_USER" "$RELAY_HOME/config/config.env"
    chmod 600 "$RELAY_HOME/config/config.env"
    echo "config template installed — edit $RELAY_HOME/config/config.env"
fi

# 10. Install nsjail config
cp "$(dirname "$0")/nsjail.cfg" "$RELAY_HOME/config/nsjail.cfg"
chown "$RELAY_USER":"$RELAY_USER" "$RELAY_HOME/config/nsjail.cfg"

# 11. Install systemd service
cp "$(dirname "$0")/oz-relay-server.service" /etc/systemd/system/
systemctl daemon-reload
echo "systemd service installed (not started — configure config.env first)"

# 12. Install Caddy config
cp "$(dirname "$0")/Caddyfile" /etc/caddy/Caddyfile
mkdir -p /var/log/caddy
echo "caddy config installed"

# 13. Install cron jobs
cp "$(dirname "$0")/relay-cron" /etc/cron.d/oz-relay
echo "cron jobs installed"

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "  1. Clone the repo:  sudo -u $RELAY_USER git clone --bare git@github.com:ozinc/arcflow-core.git $ARCFLOW_REPO/repo.git"
echo "  2. Edit config:     sudo vim $RELAY_HOME/config/config.env"
echo "  3. Build binary:    cargo build --release -p oz-relay-server"
echo "  4. Copy binary:     sudo cp target/release/oz-relay-server $RELAY_HOME/bin/"
echo "  5. Start service:   sudo systemctl enable --now oz-relay-server"
echo "  6. Start caddy:     sudo systemctl enable --now caddy"
echo "  7. Test:            curl https://relay.oz.global/.well-known/agent.json"
