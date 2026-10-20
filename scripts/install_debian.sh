#!/usr/bin/env bash
# Installation script for Debian/Ubuntu (Priority #1 target)
# Installs dbsrt-agent binary and systemd user service.
set -euo pipefail

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
SERVICE_NAME="dbsrt-agent.service"
REMOTE_API="${1:-https://api.dbsrt.perso.example.com}"

echo "=== Installing dbsrt-agent on Debian/Ubuntu ==="

# 1. Compile release binary if missing
if [[ ! -f "target/release/dbsrt-server" ]]; then
  echo "Building release binary..."
  cargo build --release --bin dbsrt-server
fi

# 2. Copy binary to ~/.local/bin/
mkdir -p "${BIN_DIR}"
cp -f "target/release/dbsrt-server" "${BIN_DIR}/dbsrt-agent"
chmod +x "${BIN_DIR}/dbsrt-agent"
echo "✓ Installed ${BIN_DIR}/dbsrt-agent"

# 3. Copy VLC Lua plugin to user extension folder
VLC_EXT_DIR="${HOME}/.local/share/vlc/lua/extensions"
mkdir -p "${VLC_EXT_DIR}"
if [[ -f "vlc-plugin/vlsub_dbsrt.lua" ]]; then
  cp -f "vlc-plugin/vlsub_dbsrt.lua" "${VLC_EXT_DIR}/vlsub_dbsrt.lua"
  echo "✓ Installed VLC extension ${VLC_EXT_DIR}/vlsub_dbsrt.lua"
fi

# 4. Create systemd user service unit
mkdir -p "${SYSTEMD_USER_DIR}"
cat <<UNIT >"${SYSTEMD_USER_DIR}/${SERVICE_NAME}"
[Unit]
Description=dbsrt Local Agent Sidecar (VLC & Video Hash Relay)
After=network.target

[Service]
Type=simple
ExecStart=${BIN_DIR}/dbsrt-agent --host 127.0.0.1 --port 8765 --agent --remote-api ${REMOTE_API} --phash-dir /tmp --subtitles-dir /tmp
Restart=always
RestartSec=3
KillMode=process
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
UNIT

echo "✓ Created systemd unit: ${SYSTEMD_USER_DIR}/${SERVICE_NAME}"

# 4. Reload systemd daemon if available
if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload || true
  echo "✓ systemd user daemon reloaded."
  echo "To enable and start:"
  echo "  systemctl --user enable --now ${SERVICE_NAME}"
fi

echo "=== Installation complete ==="
