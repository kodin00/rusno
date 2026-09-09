#!/usr/bin/env bash
set -euo pipefail

# rusno install script
# Downloads the binary, installs it, sets up the systemd service, and runs init.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --- Config ---
INSTALL_BIN="/usr/local/bin/rusno"
SERVICE_FILE="/etc/systemd/system/rusno.service"
RUSNO_USER="${RUSNO_USER:-$(whoami)}"
RUSNO_PORT="${RUSNO_PORT:-6967}"

echo "=== rusno install ==="
echo "User:       $RUSNO_USER"
echo "Binary:     $INSTALL_BIN"
echo "Port:       $RUSNO_PORT"
echo ""

# --- Check for Docker ---
if ! command -v docker &>/dev/null; then
    echo "ERROR: docker not found. Install Docker Engine + docker compose plugin first."
    exit 1
fi

if ! command -v git &>/dev/null; then
    echo "ERROR: git not found. Install git first."
    exit 1
fi

# --- Download or copy binary ---
if [[ -f "$SCRIPT_DIR/rusno" ]]; then
    echo "Copying local binary..."
    sudo cp "$SCRIPT_DIR/rusno" "$INSTALL_BIN"
elif [[ -f "$SCRIPT_DIR/target/release/rusno" ]]; then
    echo "Copying built binary..."
    sudo cp "$SCRIPT_DIR/target/release/rusno" "$INSTALL_BIN"
else
    echo "No local binary found. Please build with 'cargo build --release' or download a release."
    echo "  cargo build --release && sudo cp target/release/rusno $INSTALL_BIN"
    exit 1
fi

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
