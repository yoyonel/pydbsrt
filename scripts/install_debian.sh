#!/usr/bin/env bash
# Installation & Lifecycle Manager for Debian/Ubuntu (Priority #1 target)
# Manages dbsrt-agent binary, VLC plugin, and systemd user service.
set -euo pipefail

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
SERVICE_NAME="dbsrt-agent.service"
VLC_EXT_DIR="${HOME}/.local/share/vlc/lua/extensions"
DEFAULT_REMOTE_API="${DEFAULT_REMOTE_API:-http://127.0.0.1:8765}"
DEFAULT_PORT=8765

ACTION="${1:-install}"
if [[ "${ACTION}" =~ ^https?:// ]]; then
  REMOTE_API="${ACTION}"
  ACTION="install"
else
  REMOTE_API="${2:-${DEFAULT_REMOTE_API}}"
fi

case "${ACTION}" in
  install)
    echo "=== Installing dbsrt-agent on Debian/Ubuntu ==="
    echo "Configuring Remote API Target: ${REMOTE_API}"

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
ExecStart=${BIN_DIR}/dbsrt-agent --host 127.0.0.1 --port ${DEFAULT_PORT} --agent --remote-api ${REMOTE_API} --phash-dir /tmp --subtitles-dir /tmp
Restart=always
RestartSec=3
KillMode=process
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
UNIT

    echo "✓ Created systemd unit: ${SYSTEMD_USER_DIR}/${SERVICE_NAME}"

    # 5. Reload and start systemd user service
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user daemon-reload
      systemctl --user enable --now "${SERVICE_NAME}"
      echo "✓ Service ${SERVICE_NAME} enabled and started."

      # Healthcheck
      echo "Waiting for healthcheck on http://127.0.0.1:${DEFAULT_PORT}/v1/health..."
      for _ in {1..20}; do
        if curl -s -f "http://127.0.0.1:${DEFAULT_PORT}/v1/health" >/dev/null 2>&1; then
          echo "✓ Local agent sidecar is active and healthy on port ${DEFAULT_PORT}!"
          exit 0
        fi
        sleep 0.2
      done
      echo "⚠️ Local agent did not respond within 4s, checking journalctl:"
      journalctl --user -u "${SERVICE_NAME}" -n 15 --no-pager
    fi
    ;;

  start)
    echo "Starting ${SERVICE_NAME}..."
    systemctl --user start "${SERVICE_NAME}"
    systemctl --user status "${SERVICE_NAME}" --no-pager
    ;;

  stop)
    echo "Stopping ${SERVICE_NAME}..."
    systemctl --user stop "${SERVICE_NAME}"
    echo "✓ ${SERVICE_NAME} stopped."
    ;;

  restart)
    echo "Restarting ${SERVICE_NAME}..."
    systemctl --user restart "${SERVICE_NAME}"
    systemctl --user status "${SERVICE_NAME}" --no-pager
    ;;

  status)
    echo "=== systemd user status (${SERVICE_NAME}) ==="
    systemctl --user status "${SERVICE_NAME}" --no-pager || true
    echo ""
    echo "=== Local Agent API Status (http://127.0.0.1:${DEFAULT_PORT}/v1/status) ==="
    if curl -s -f "http://127.0.0.1:${DEFAULT_PORT}/v1/status" 2>/dev/null; then
      echo ""
    else
      echo "Local agent is not responding on port ${DEFAULT_PORT}."
    fi
    ;;

  logs)
    journalctl --user -u "${SERVICE_NAME}" -n 50 -f
    ;;

  uninstall)
    echo "=== Uninstalling dbsrt-agent and VLC plugin ==="
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user stop "${SERVICE_NAME}" 2>/dev/null || true
      systemctl --user disable "${SERVICE_NAME}" 2>/dev/null || true
      rm -f "${SYSTEMD_USER_DIR}/${SERVICE_NAME}"
      systemctl --user daemon-reload
      echo "✓ Removed systemd user unit."
    fi
    rm -f "${BIN_DIR}/dbsrt-agent"
    echo "✓ Removed binary ${BIN_DIR}/dbsrt-agent."
    rm -f "${VLC_EXT_DIR}/vlsub_dbsrt.lua"
    echo "✓ Removed VLC extension ${VLC_EXT_DIR}/vlsub_dbsrt.lua."
    echo "✓ Complete uninstallation finished."
    ;;

  *)
    echo "Usage: $0 {install [REMOTE_URL]|start|stop|restart|status|logs|uninstall}"
    exit 1
    ;;
esac
