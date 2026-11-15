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
EMPTY_RELAY_DATA_DIR="/tmp"
RESTART_DELAY_SECS=3
HEALTHCHECK_MAX_ATTEMPTS=20
HEALTHCHECK_POLL_INTERVAL_SECS="0.2"
JOURNAL_FAILURE_LINES=15
JOURNAL_FOLLOW_LINES=50
KEY_FILE_UMASK="077"
KEY_FILE_PERMS="600"

print_usage() {
  echo "Usage: $(basename "$0") {install [REMOTE_URL] [--with-local-catalog] [--on-demand] [--skip-build]|start|stop|restart|on-demand|status|logs|uninstall} [--help]"
  echo ""
  echo "Manages dbsrt-agent binary, VLC plugin, and systemd user service on Debian/Ubuntu."
  echo "By default, --agent runs in frugal zero-catalog relay mode (~6-8 MiB RSS, 0 local datasets mounted)."
  echo ""
  echo "Actions:"
  echo "  install [URL]          Install agent binary, VLC extension and systemd user unit"
  echo "    --with-local-catalog Explicitly mount local data/__PHASH__, catalog.dbphash & data/SRT"
  echo "    --on-demand          Install unit disabled & stopped (0 MiB idle; start explicitly when needed)"
  echo "    --skip-build         Reuse pre-built target/release/dbsrt-server without invoking cargo"
  echo "  start                  Start systemd user service"
  echo "  stop                   Stop systemd user service (frees 100% RAM immediately)"
  echo "  restart                Update binary (if built) and restart systemd user service"
  echo "  on-demand              Stop and disable auto-start at boot (0 MiB idle until 'start')"
  echo "  status                 Show systemd unit and local agent health (default)"
  echo "  logs                   Follow journalctl logs"
  echo "  uninstall              Stop service and remove all installed files"
}

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || "${1:-}" == "help" ]]; then
  print_usage
  exit 0
fi

ACTION="status"
REMOTE_API=""
WITH_LOCAL_CATALOG=0
ON_DEMAND=0
SKIP_BUILD=0

if [[ $# -gt 0 ]]; then
  if [[ "${1}" =~ ^https?:// ]]; then
    ACTION="install"
    REMOTE_API="${1}"
    shift
  else
    ACTION="${1}"
    shift
  fi
fi

while [[ $# -gt 0 ]]; do
  case "${1}" in
    --with-local-catalog)
      WITH_LOCAL_CATALOG=1
      ;;
    --on-demand | --no-enable)
      ON_DEMAND=1
      ;;
    --skip-build)
      SKIP_BUILD=1
      ;;
    --help | -h)
      print_usage
      exit 0
      ;;
    https://* | http://*)
      REMOTE_API="${1}"
      ;;
    *)
      if [[ -z "${REMOTE_API}" ]]; then
        REMOTE_API="${1}"
      else
        echo "❌ Unknown argument: ${1}" >&2
        print_usage >&2
        exit 1
      fi
      ;;
  esac
  shift
done

if [[ -z "${REMOTE_API}" ]]; then
  REMOTE_API="${O2SWITCH_URL:-}"
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

    # 1. Build up-to-date release binary when Cargo workspace & toolchain exist on host
    if [[ "${SKIP_BUILD}" -eq 0 && -f "${REPO_ROOT}/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1; then
      echo "Ensuring up-to-date release binary (cargo build --release --bin dbsrt-server)..."
      cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" --bin dbsrt-server
    elif [[ ! -f "${REPO_ROOT}/target/release/dbsrt-server" && ! -x "${REPO_ROOT}/target/debug/dbsrt-server" ]]; then
      echo "❌ Pre-built dbsrt-server binary not found and cargo toolchain unavailable." >&2
      exit 1
    fi

    # 2. Copy binary to ~/.local/bin/ (and sync companion dbsrt/dbsrt-server if already present)
    mkdir -p "${BIN_DIR}"
    local_bin="${REPO_ROOT}/target/release/dbsrt-server"
    if [[ ! -f "${local_bin}" && -f "${REPO_ROOT}/target/debug/dbsrt-server" ]]; then
      local_bin="${REPO_ROOT}/target/debug/dbsrt-server"
    fi
    cp -f "${local_bin}" "${BIN_DIR}/dbsrt-agent"
    chmod +x "${BIN_DIR}/dbsrt-agent"
    echo "✓ Installed ${BIN_DIR}/dbsrt-agent"
    if [[ -f "${BIN_DIR}/dbsrt-server" ]]; then
      cp -f "${local_bin}" "${BIN_DIR}/dbsrt-server"
      chmod +x "${BIN_DIR}/dbsrt-server"
      echo "✓ Synchronized ${BIN_DIR}/dbsrt-server"
    fi
    if [[ -f "${BIN_DIR}/dbsrt" && -f "${REPO_ROOT}/target/release/dbsrt" ]]; then
      cp -f "${REPO_ROOT}/target/release/dbsrt" "${BIN_DIR}/dbsrt"
      chmod +x "${BIN_DIR}/dbsrt"
      echo "✓ Synchronized ${BIN_DIR}/dbsrt"
    fi

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
      (umask "${KEY_FILE_UMASK}" && printf '%s' "${API_KEY}" >"${AGENT_KEY_FILE}")
      chmod "${KEY_FILE_PERMS}" "${AGENT_KEY_FILE}"
      CREDENTIAL_LINE="LoadCredential=dbsrt_remote_api_key:${AGENT_KEY_FILE}"
      echo "✓ Stored remote API key in ${AGENT_KEY_FILE} (mode ${KEY_FILE_PERMS})"
    else
      rm -f "${AGENT_KEY_FILE}"
      echo "⚠️ No remote API key in DBSRT_REMOTE_API_KEY or 'pass': protected remotes answer 401."
    fi

    # 5. Frugal zero-catalog relay by default: NEVER mount multi-GiB local catalogs on --agent
    #    unless --with-local-catalog is explicitly requested by the operator.
    LOCAL_DATA_ARGS=" --phash-dir ${EMPTY_RELAY_DATA_DIR} --subtitles-dir ${EMPTY_RELAY_DATA_DIR}"
    if [[ "${WITH_LOCAL_CATALOG}" -eq 1 ]]; then
      LOCAL_DATA_ARGS=""
      if [[ -d "${REPO_ROOT}/data/__PHASH__" ]]; then
        LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --phash-dir ${REPO_ROOT}/data/__PHASH__"
      else
        LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --phash-dir ${EMPTY_RELAY_DATA_DIR}"
      fi
      if [[ -f "${REPO_ROOT}/data/catalog.dbphash" ]]; then
        LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --dbphash ${REPO_ROOT}/data/catalog.dbphash"
      fi
      if [[ -d "${REPO_ROOT}/data/SRT" ]]; then
        LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --subtitles-dir ${REPO_ROOT}/data/SRT"
      else
        LOCAL_DATA_ARGS="${LOCAL_DATA_ARGS} --subtitles-dir ${EMPTY_RELAY_DATA_DIR}"
      fi
      echo "ℹ️ Local catalog mode enabled (--with-local-catalog)."
    else
      echo "✓ Frugal zero-catalog relay mode active (0 local datasets mounted)."
    fi

    # 6. Create systemd user service unit
    mkdir -p "${SYSTEMD_USER_DIR}"
    cat <<UNIT >"${SYSTEMD_USER_DIR}/${SERVICE_NAME}"
[Unit]
Description=dbsrt Local Agent Sidecar (Frugal VLC & Video Hash Relay)
After=network.target

[Service]
Type=simple
${CREDENTIAL_LINE}
ExecStart=${BIN_DIR}/dbsrt-agent --host 127.0.0.1 --port ${DEFAULT_PORT} --agent --remote-api ${REMOTE_API}${LOCAL_DATA_ARGS}
Restart=always
RestartSec=${RESTART_DELAY_SECS}
KillMode=process
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
UNIT

    echo "✓ Created systemd unit: ${SYSTEMD_USER_DIR}/${SERVICE_NAME}"

    # 7. Reload systemd user manager (either on-demand 0 MiB idle or enabled + active)
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user daemon-reload
      if [[ "${ON_DEMAND}" -eq 1 ]]; then
        systemctl --user stop "${SERVICE_NAME}" 2>/dev/null || true
        systemctl --user disable "${SERVICE_NAME}" 2>/dev/null || true
        echo "✓ Installed in on-demand mode (${SERVICE_NAME} disabled & stopped -> 0 MiB idle RAM)."
        echo "  Start only when needed: task client:linux:start (stop via: task client:linux:stop)"
        exit 0
      fi

      systemctl --user enable "${SERVICE_NAME}"
      # restart, not start: a reinstall must reload the new unit and key
      systemctl --user restart "${SERVICE_NAME}"
      echo "✓ Service ${SERVICE_NAME} enabled and started."

      # Healthcheck
      echo "Waiting for healthcheck on http://127.0.0.1:${DEFAULT_PORT}/v1/health..."
      for ((attempt = 1; attempt <= HEALTHCHECK_MAX_ATTEMPTS; attempt++)); do
        if curl -s -f "http://127.0.0.1:${DEFAULT_PORT}/v1/health" >/dev/null 2>&1; then
          echo "✓ Local agent sidecar is active and healthy on port ${DEFAULT_PORT}!"
          exit 0
        fi
        sleep "${HEALTHCHECK_POLL_INTERVAL_SECS}"
      done
      echo "⚠️ Local agent did not respond within timeout, checking journalctl:"
      journalctl --user -u "${SERVICE_NAME}" -n "${JOURNAL_FAILURE_LINES}" --no-pager
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
    echo "✓ ${SERVICE_NAME} stopped (0 MiB RAM allocated)."
    ;;

  on-demand | disable)
    echo "Switching ${SERVICE_NAME} to on-demand mode (stopped & disabled at boot)..."
    systemctl --user stop "${SERVICE_NAME}" 2>/dev/null || true
    systemctl --user disable "${SERVICE_NAME}" 2>/dev/null || true
    echo "✓ ${SERVICE_NAME} is now stopped and disabled at boot (0 MiB idle RAM)."
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
    journalctl --user -u "${SERVICE_NAME}" -n "${JOURNAL_FOLLOW_LINES}" -f
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
    print_usage >&2
    exit 1
    ;;
esac
