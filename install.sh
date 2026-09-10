#!/bin/sh
# Install rusno from the latest GitHub release (Linux x86_64).
# Downloads the binary, installs it, sets up the systemd service, and runs init.
#
# The release workflow bakes the real repo into the __REPO__ placeholder; run
# locally from a checkout with: REPO=owner/repo ./install.sh
set -eu

REPO="${REPO:-__REPO__}"
INSTALL_DIR="${RUSNO_INSTALL_DIR:-/usr/local/bin}"
BIN="$INSTALL_DIR/rusno"
# Run the service as the invoking user so it owns ~/.rusno. SUDO_USER covers
# `curl ... | sudo sh`.
run_user="${SUDO_USER:-${USER:-$(id -un)}}"
RUSNO_PORT="${RUSNO_PORT:-6967}"

case "$(uname -s)" in
  Linux) ;;
  *) echo "rusno: unsupported OS: $(uname -s) (Linux only)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  *) echo "rusno: unsupported architecture: $(uname -m) (x86_64 only)" >&2; exit 1 ;;
esac

# --- Check for dependencies ---
if ! command -v docker >/dev/null 2>&1; then
  echo "rusno: docker not found. Install Docker Engine + docker compose plugin first." >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "rusno: git not found. Install git first." >&2
  exit 1
fi

asset="rusno-linux-$arch"
base="https://github.com/$REPO/releases/latest/download"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "rusno: downloading $asset ..."
curl -fsSL "$base/$asset" -o "$tmp/$asset"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"

# Verify the download against the published checksum.
expected="$(awk -v a="$asset" '$2 == a { print $1; exit }' "$tmp/SHA256SUMS")"
actual="$(sha256sum "$tmp/$asset" | awk '{ print $1 }')"
if [ -n "$expected" ] && [ "$expected" != "$actual" ]; then
  echo "rusno: checksum mismatch for $asset (corrupt download?)" >&2
  exit 1
fi

echo "rusno: installing to $BIN ..."
if [ -w "$INSTALL_DIR" ]; then
  install -m 0755 "$tmp/$asset" "$BIN"
else
  sudo install -m 0755 "$tmp/$asset" "$BIN"
fi

# --- Initialize rusno ---
echo "rusno: running init (port $RUSNO_PORT) ..."
rusno init --port "$RUSNO_PORT"

# Set up (or refresh) the systemd service when systemd is available.
if command -v systemctl >/dev/null 2>&1; then
  echo "rusno: configuring systemd service ..."
  sudo tee /etc/systemd/system/rusno.service >/dev/null <<EOF
[Unit]
Description=rusno — self-hosted deployment manager
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
Type=simple
User=${run_user}
Group=docker
ExecStart=${BIN} serve --port ${RUSNO_PORT}
Restart=on-failure
RestartSec=5
Environment=RUSNO_HOME=${HOME:-/home/${run_user}}/.rusno

[Install]
WantedBy=multi-user.target
EOF
  sudo systemctl daemon-reload
  sudo systemctl enable rusno
  sudo systemctl restart rusno
  echo "rusno: service running on http://localhost:${RUSNO_PORT}"
else
  echo "rusno: systemd not detected; run '${BIN} serve --port ${RUSNO_PORT}' under your own supervisor" >&2
fi

echo "rusno: installed. $("$BIN" --version 2>/dev/null || echo ok)"
echo ""
echo "IMPORTANT: Back up your master key at ~/.rusno/keys/master.key"
echo "           Losing it means all encrypted secrets are unrecoverable."
