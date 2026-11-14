#!/usr/bin/env bash
# Installation & Lifecycle Manager for Debian/Ubuntu (Priority #1 target)
# Manages dbsrt-agent binary, VLC plugin, and systemd user service.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Load optional unversioned local environment file (O2SWITCH_URL) and resolve_api_key
# shellcheck source=scripts/infra/local/load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_local_env "${REPO_ROOT}"

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
SERVICE_NAME="dbsrt-agent.service"
VLC_EXT_DIR="${HOME}/.local/share/vlc/lua/extensions"
# Holds the remote API key (mode 0600): argv and the unit file are world-readable
AGENT_KEY_FILE="${HOME}/.config/pydbsrt/dbsrt-remote-api-key"
# Former EnvironmentFile holding the key in clear: removed on install and uninstall
LEGACY_ENV_FILE="${HOME}/.config/pydbsrt/dbsrt-agent.env"
DEFAULT_PORT=8765

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || "${1:-}" == "help" ]]; then
  echo "Usage: $(basename "$0") {install [REMOTE_URL]|start|stop|restart|status|logs|uninstall} [--help]"
  echo ""
  echo "Manages dbsrt-agent binary, VLC plugin, and systemd user service on Debian/Ubuntu."
  echo ""
  echo "Actions:"
  echo "  install [URL]  Install agent binary, VLC extension and systemd service"
  echo "  start          Start systemd user service"
  echo "  stop           Stop systemd user service"
  echo "  restart        Restart systemd user service"
  echo "  status         Show systemd unit and local agent health (default)"
  echo "  logs           Follow journalctl logs"
  echo "  uninstall      Stop service and remove all installed files"
  exit 0
fi

ACTION="${1:-status}"
if [[ "${ACTION}" =~ ^https?:// ]]; then
  REMOTE_API="${ACTION}"
  ACTION="install"
else
  REMOTE_API="${2:-${O2SWITCH_URL:-}}"
fi

case "${ACTION}" in
  install)
    echo "=== Installing dbsrt-agent on Debian/Ubuntu ==="
    if [[ -z "${REMOTE_API}" ]]; then
      echo "❌ No remote API: pass REMOTE_URL or set O2SWITCH_URL (env or .env)." >&2
      exit 1
    fi
    if [[ "${REMOTE_API}" =~ ^https?://(127\.0\.0\.1|localhost|\[::1\]):${DEFAULT_PORT}(/|$) ]]; then
      echo "❌ Remote API ${REMOTE_API} is the agent itself: it would relay to itself." >&2
      exit 1
    fi
    echo "Configuring Remote API Target: ${REMOTE_API}"

    # 1. Compile release binary if missing
    if [[ ! -f "${REPO_ROOT}/target/release/dbsrt-server" ]]; then
      if [[ -x "${REPO_ROOT}/target/debug/dbsrt-server" ]]; then
        echo "Using debug binary ${REPO_ROOT}/target/debug/dbsrt-server..."
      else
        echo "Building release binary..."
        cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" --bin dbsrt-server
      fi
    fi

    # 2. Copy binary to ~/.local/bin/
    mkdir -p "${BIN_DIR}"
    local_bin="${REPO_ROOT}/target/release/dbsrt-server"
    if [[ ! -f "${local_bin}" && -f "${REPO_ROOT}/target/debug/dbsrt-server" ]]; then
      local_bin="${REPO_ROOT}/target/debug/dbsrt-server"
    fi
    cp -f "${local_bin}" "${BIN_DIR}/dbsrt-agent"
    chmod +x "${BIN_DIR}/dbsrt-agent"
    echo "✓ Installed ${BIN_DIR}/dbsrt-agent"

    # 3. Copy VLC Lua plugin to user extension folder
    mkdir -p "${VLC_EXT_DIR}"
    if [[ -f "${REPO_ROOT}/vlc-plugin/vlsub_dbsrt.lua" ]]; then
      cp -f "${REPO_ROOT}/vlc-plugin/vlsub_dbsrt.lua" "${VLC_EXT_DIR}/vlsub_dbsrt.lua"
      echo "✓ Installed VLC extension ${VLC_EXT_DIR}/vlsub_dbsrt.lua"
    fi

    # 4. Write the remote API key to a private file, handed over by systemd LoadCredential=:
    #    unlike an env var, the agent's ffmpeg children never inherit it
    rm -f "${LEGACY_ENV_FILE}"
    API_KEY="$(resolve_api_key)"
    CREDENTIAL_LINE=""
    if [[ -n "${API_KEY}" ]]; then
      mkdir -p "$(dirname "${AGENT_KEY_FILE}")"
      (umask 077 && printf '%s' "${API_KEY}" >"${AGENT_KEY_FILE}")
      chmod 600 "${AGENT_KEY_FILE}"
      CREDENTIAL_LINE="LoadCredential=dbsrt_remote_api_key:${AGENT_KEY_FILE}"
      echo "✓ Stored remote API key in ${AGENT_KEY_FILE} (mode 600)"
    else
      rm -f "${AGENT_KEY_FILE}"
      echo "⚠️ No remote API key in DBSRT_REMOTE_API_KEY or 'pass': protected remotes answer 401."
    fi

    # 5. Build local dataset arguments if present in repository (omit --ahash-dir on lightweight --agent)
    LOCAL_DATA_ARGS=""
    if [[ -d "${REPO_ROOT}/data/__PHASH__" ]]; then
      LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --phash-dir ${REPO_ROOT}/data/__PHASH__"
    else
      LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --phash-dir /tmp"
    fi
    if [[ -f "${REPO_ROOT}/data/catalog.dbphash" ]]; then
      LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --dbphash ${REPO_ROOT}/data/catalog.dbphash"
    fi
    if [[ -d "${REPO_ROOT}/data/SRT" ]]; then
      LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --subtitles-dir ${REPO_ROOT}/data/SRT"
    else
      LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --subtitles-dir /tmp"
    fi

    # 6. Create systemd user service unit
    mkdir -p "${SYSTEMD_USER_DIR}"
    cat <<UNIT >"${SYSTEMD_USER_DIR}/${SERVICE_NAME}"
[Unit]
Description=dbsrt Local Agent Sidecar (VLC & Video Hash Relay)
After=network.target

[Service]
Type=simple
${CREDENTIAL_LINE}
ExecStart=${BIN_DIR}/dbsrt-agent --host 127.0.0.1 --port ${DEFAULT_PORT} --agent --remote-api ${REMOTE_API}${LOCAL_DATA_ARGS}
Restart=always
RestartSec=3
KillMode=process
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
UNIT

    echo "✓ Created systemd unit: ${SYSTEMD_USER_DIR}/${SERVICE_NAME}"

    # 7. Reload and start systemd user service
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user daemon-reload
      systemctl --user enable "${SERVICE_NAME}"
      # restart, not start: a reinstall must reload the new unit and key
      systemctl --user restart "${SERVICE_NAME}"
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
    local_bin="${REPO_ROOT}/target/release/dbsrt-server"
    if [[ ! -f "${local_bin}" && -f "${REPO_ROOT}/target/debug/dbsrt-server" ]]; then
      local_bin="${REPO_ROOT}/target/debug/dbsrt-server"
    fi
    if [[ -f "${local_bin}" ]]; then
      cp -f "${local_bin}" "${BIN_DIR}/dbsrt-agent"
      chmod +x "${BIN_DIR}/dbsrt-agent"
    fi
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
    rm -f "${BIN_DIR}/dbsrt-agent" "${AGENT_KEY_FILE}" "${LEGACY_ENV_FILE}"
    echo "✓ Removed binary ${BIN_DIR}/dbsrt-agent and key file ${AGENT_KEY_FILE}."
    rm -f "${VLC_EXT_DIR}/vlsub_dbsrt.lua"
    echo "✓ Removed VLC extension ${VLC_EXT_DIR}/vlsub_dbsrt.lua."
    echo "✓ Complete uninstallation finished."
    ;;

  *)
    echo "Usage: $(basename "$0") {install [REMOTE_URL]|start|stop|restart|status|logs|uninstall}"
    exit 1
    ;;
esac
