#!/usr/bin/env bash
set -euo pipefail

# rusno install script
# Installs the latest rusno binary, sets up the systemd service, and runs init.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/kodin00/rusno/master/install.sh | sudo bash
#   ./install.sh                       # use a locally built binary if present
#   RUSNO_PORT=7000 ./install.sh       # override the listen port

REPO="kodin00/rusno"
ASSET="rusno-x86_64-linux.tar.gz"
RELEASE_URL="https://github.com/${REPO}/releases/download/latest/${ASSET}"

INSTALL_BIN="/usr/local/bin/rusno"
SERVICE_FILE="/etc/systemd/system/rusno.service"
RUSNO_USER="${RUSNO_USER:-$(whoami)}"
RUSNO_PORT="${RUSNO_PORT:-6967}"

# Script's own dir (empty when piped to bash via curl)
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
else
    SCRIPT_DIR=""
fi

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "=== rusno install ==="
echo "User:       $RUSNO_USER"
echo "Binary:     $INSTALL_BIN"
echo "Port:       $RUSNO_PORT"
echo ""

# --- Check for dependencies ---
if ! command -v docker &>/dev/null; then
    echo "ERROR: docker not found. Install Docker Engine + docker compose plugin first."
    exit 1
fi

if ! command -v git &>/dev/null; then
    echo "ERROR: git not found. Install git first."
    exit 1
fi

# --- Resolve the binary: local build, local file, or download latest ---
RUSNO_BIN=""

if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/rusno" ]]; then
    echo "Using local binary at $SCRIPT_DIR/rusno"
    RUSNO_BIN="$SCRIPT_DIR/rusno"
elif [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/target/release/rusno" ]]; then
    echo "Using locally built binary..."
    RUSNO_BIN="$SCRIPT_DIR/target/release/rusno"
else
    echo "Downloading latest release from GitHub..."
    if ! command -v curl &>/dev/null; then
        echo "ERROR: curl not found. Install curl or download $RELEASE_URL manually."
        exit 1
    fi
    if ! curl -fsSL "$RELEASE_URL" -o "$TMPDIR/$ASSET"; then
        echo "ERROR: failed to download $RELEASE_URL"
        echo "       Check that a release exists under the 'latest' tag, or build locally"
        echo "       with 'cargo build --release' and re-run ./install.sh."
        exit 1
    fi
    tar xzf "$TMPDIR/$ASSET" -C "$TMPDIR"
    RUSNO_BIN="$TMPDIR/rusno"
    if [[ ! -f "$RUSNO_BIN" ]]; then
        echo "ERROR: binary not found in archive after extraction."
        exit 1
    fi
fi

echo "Installing binary..."
sudo cp "$RUSNO_BIN" "$INSTALL_BIN"
sudo chmod +x "$INSTALL_BIN"

# --- Initialize rusno ---
echo ""
echo "Running rusno init..."
rusno init --port "$RUSNO_PORT"

# --- Install systemd service ---
SERVICE_CONTENT=$(cat <<EOF
[Unit]
Description=rusno — self-hosted deployment manager
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
Type=simple
User=$RUSNO_USER
Group=docker
ExecStart=$INSTALL_BIN serve --port $RUSNO_PORT
Restart=on-failure
RestartSec=5
Environment=RUSNO_HOME=$HOME/.rusno

[Install]
WantedBy=multi-user.target
EOF
)

echo ""
echo "Installing systemd service..."
echo "$SERVICE_CONTENT" | sudo tee "$SERVICE_FILE" > /dev/null
sudo systemctl daemon-reload
sudo systemctl enable rusno

echo ""
echo "=== Installation complete ==="
echo ""
echo "Start rusno:"
echo "  sudo systemctl start rusno"
echo ""
echo "Check status:"
echo "  sudo systemctl status rusno"
echo ""
echo "Then visit http://localhost:$RUSNO_PORT to complete setup."
echo ""
echo "IMPORTANT: Back up your master key at ~/.rusno/keys/master.key"
echo "           Losing it means all encrypted secrets are unrecoverable."
