#!/usr/bin/env bash
# Switches the active backend endpoint for the local VLC client
# between: 1) Local Zero-DB server, 2) Cloud o2switch relay, 3) Raspberry Pi 4 appliance.
set -euo pipefail

# Load optional unversioned local environment file (O2SWITCH_URL, cloud relay base URL)
# shellcheck source=scripts/load_env.sh
source "$(dirname "${BASH_SOURCE[0]}")/load_env.sh"
load_local_env "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Named operational constants
DEFAULT_TARGET_PORT=8765
DEFAULT_RPI4_HOST="rpi4"
DISCOVERY_FILE="/tmp/dbsrt-server.json"
PID_RELAY="target/dbsrt-relay-agent.pid"
PID_LOCAL="/tmp/dbsrt-server.pid"

MODE="${1:-status}"
PORT="${TARGET_PORT:-${DEFAULT_TARGET_PORT}}"
RPI_HOST="${RPI4_HOST:-${DEFAULT_RPI4_HOST}}"

resolve_rpi_target() {
  local host="${1:-rpi4}"
  local ip
  ip=$(ssh -G "${host}" 2>/dev/null | awk '/^hostname / {print $2}' || true)
  if [[ -n "${ip}" ]]; then
    echo "${ip}"
  else
    echo "${host}"
  fi
}

stop_all_daemons() {
  if [[ -f "${PID_RELAY}" ]]; then
    ./scripts/agent_relay.sh stop >/dev/null 2>&1 || true
  fi
  if [[ -f "${PID_LOCAL}" ]]; then
    ./scripts/stack_ctl.sh server-stop >/dev/null 2>&1 || true
  fi
  # Kill any stray process bound to PORT
  fuser -k "${PORT}/tcp" >/dev/null 2>&1 || true
  rm -f "${DISCOVERY_FILE}"
}

check_active_mode() {
  if ! curl -s -f "http://127.0.0.1:${PORT}/v1/health" >/dev/null 2>&1; then
    echo "INACTIVE (No daemon listening on 127.0.0.1:${PORT})"
    return 1
  fi

  local pid
  pid=$(jq -r '.pid // empty' "${DISCOVERY_FILE}" 2>/dev/null || true)
  if [[ -n "${pid}" ]]; then
    local cmd
    cmd=$(ps -p "${pid}" -o args= 2>/dev/null || true)
    if echo "${cmd}" | grep -q -- "--remote-api"; then
      local remote_url
      remote_url=$(echo "${cmd}" | sed -n 's/.*--remote-api \([^ ]*\).*/\1/p')
      if [[ -n "${O2SWITCH_URL:-}" && "${remote_url}" == "${O2SWITCH_URL}"* ]]; then
        echo "CLOUD_O2SWITCH (Relay -> ${remote_url})"
      elif echo "${remote_url}" | grep -q "${RPI_HOST}"; then
        echo "RPI4_RELAY (Relay -> ${remote_url})"
      else
        echo "AGENT_RELAY (Relay -> ${remote_url})"
      fi
      return 0
    fi
  fi

  local status_json
  status_json=$(curl -s "http://127.0.0.1:${PORT}/v1/status" 2>/dev/null || echo "{}")
  local indexed_count
  indexed_count=$(echo "${status_json}" | jq -r '.indexed_phash_count // 0' 2>/dev/null || echo "0")
  echo "LOCAL_STANDALONE (Zero-DB / RAM MIH, ${indexed_count} indexed fingerprints)"
}

case "${MODE}" in
  status)
    echo "=== Active VLC Backend Endpoint Status ==="
    echo -n "• Current Mode     : "
    check_active_mode || true
    echo "• VLC Discovery    : ${DISCOVERY_FILE}"
    if [[ -f "${DISCOVERY_FILE}" ]]; then
      echo "  $(cat "${DISCOVERY_FILE}")"
    else
      echo "  (No discovery file found)"
    fi
    echo "• VLC Config Target: http://127.0.0.1:${PORT}"
    ;;

  local)
    echo "=== Switching to 100% LOCAL STANDALONE Mode (Zero-DB / RAM) ==="
    stop_all_daemons
    ./scripts/stack_ctl.sh server-start
    echo "✓ Active mode: LOCAL_STANDALONE"
    echo "  VLC communicates with local daemon on http://127.0.0.1:${PORT}"
    echo "  Subtitles and pHash resolved entirely from local disk & RAM."
    ;;

  o2switch)
    echo "=== Switching to CLOUD O2SWITCH Mode (Remote Relay) ==="
    stop_all_daemons
    ./scripts/agent_relay.sh start
    echo "✓ Active mode: CLOUD_O2SWITCH"
    echo "  VLC communicates with local agent on http://127.0.0.1:${PORT}"
    echo "  Visual pHash fingerprints forwarded via HTTPS to ${O2SWITCH_URL:-<O2SWITCH_URL unset>}"
    ;;

  rpi4)
    echo "=== Switching to RASPBERRY PI 4 Appliance Mode ==="
    stop_all_daemons
    target_ip=$(resolve_rpi_target "${RPI_HOST}")
    echo "Starting local agent relaying to RPi4 (http://${target_ip}:${PORT})..."
    O2SWITCH_URL="http://${target_ip}:${PORT}" ./scripts/agent_relay.sh start
    echo "✓ Active mode: RPI4_APPLIANCE"
    echo "  VLC communicates with local agent on http://127.0.0.1:${PORT}"
    echo "  Fingerprints relayed over LAN to http://${target_ip}:${PORT}"
    ;;

  *)
    echo "Usage: $0 {status|local|o2switch|rpi4}"
    exit 1
    ;;
esac
