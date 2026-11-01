#!/usr/bin/env bash
# Controls the local dbsrt client relay agent connected to Cloud o2switch
set -euo pipefail

# Load optional unversioned local environment file (O2SWITCH_URL)
# shellcheck source=scripts/load_env.sh
source "$(dirname "${BASH_SOURCE[0]}")/load_env.sh"
load_local_env "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Named operational constants
DEFAULT_AGENT_PORT=8765
DEFAULT_HEALTHCHECK_MAX_RETRIES=15
DEFAULT_HEALTHCHECK_SLEEP_SEC=0.5
DEFAULT_SHUTDOWN_TIMEOUT_SEC=10
DEFAULT_LOCAL_SNIPPET="data/snippets/Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4"
PID_FILE="target/dbsrt-relay-agent.pid"
LOG_FILE="target/dbsrt-relay-agent.log"
SERVER_BIN="target/release/dbsrt-server"

ACTION="${1:-status}"
PORT="${AGENT_PORT:-${DEFAULT_AGENT_PORT}}"
REMOTE_API="${O2SWITCH_URL:-}"

resolve_sample_video() {
  local candidates=(
    "${SYNOLOGY_TVSHOW_DIR:-}/Andor/Andor.S01E01.1080p.WEB.h264-KOGi.mkv"
    "/media/${USER}/Synology/tvshow/Andor/Andor.S01E01.1080p.WEB.h264-KOGi.mkv"
    "${HOME}/__SYNOLOGY__/tvshow/Andor/Andor.S01E01.1080p.WEB.h264-KOGi.mkv"
    "/mnt/synology/tvshow/Andor/Andor.S01E01.1080p.WEB.h264-KOGi.mkv"
    "${DEFAULT_LOCAL_SNIPPET}"
  )
  for c in "${candidates[@]}"; do
    if [[ -n "${c}" && -f "${c}" ]]; then
      echo "${c}"
      return 0
    fi
  done
  echo "${DEFAULT_LOCAL_SNIPPET}"
}

case "${ACTION}" in
  start)
    mkdir -p target
    if [[ -f "${PID_FILE}" ]]; then
      EXISTING_PID=$(cat "${PID_FILE}")
      if kill -0 "${EXISTING_PID}" 2>/dev/null; then
        echo "⚠️ dbsrt-relay-agent is already running (PID: ${EXISTING_PID})."
        exit 0
      fi
    fi

    if [[ ! -f "${SERVER_BIN}" ]]; then
      echo "=== Building release binary: dbsrt-server ==="
      cargo build --release --bin dbsrt-server
    fi

    if [[ -z "${REMOTE_API}" ]]; then
      echo "❌ O2SWITCH_URL is not set (env or .env), e.g. https://<your-domain>/dbsrt"
      exit 1
    fi

    API_KEY=$(resolve_api_key)
    if [[ -z "${API_KEY}" ]]; then
      echo "⚠️ Warning: No remote API key found in 'pass' or DBSRT_REMOTE_API_KEY."
      echo "   Requests to protected Cloud o2switch might fail with 401 Unauthorized."
    fi

    echo "=== Starting local dbsrt relay agent on 127.0.0.1:${PORT} ==="
    echo "    Remote Cloud Relay Target : ${REMOTE_API}"
    echo "    VLC Service Discovery     : /tmp/dbsrt-server.json"

    # Key goes through the environment: argv is readable by any local user via ps
    if [[ -n "${API_KEY}" ]]; then
      export DBSRT_REMOTE_API_KEY="${API_KEY}"
    fi

    setsid "${SERVER_BIN}" \
      --host 127.0.0.1 \
      --port "${PORT}" \
      --agent \
      --remote-api "${REMOTE_API}" </dev/null >>"${LOG_FILE}" 2>&1 &
    AGENT_PID=$!
    echo "${AGENT_PID}" >"${PID_FILE}"

    echo "Waiting for healthcheck on http://127.0.0.1:${PORT}/v1/health..."
    HEALTHY=false
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      if curl -s -f "http://127.0.0.1:${PORT}/v1/health" >/dev/null 2>&1; then
        HEALTHY=true
        break
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done

    if [[ "${HEALTHY}" != "true" ]]; then
      echo "❌ Relay agent healthcheck failed. Tail of log:"
      tail -n 20 "${LOG_FILE}"
      exit 1
    fi
    echo "✓ Local relay agent running with PID $(cat "${PID_FILE}")."
    ;;

  stop)
    if [[ ! -f "${PID_FILE}" ]]; then
      echo "No PID file found at ${PID_FILE}."
      exit 0
    fi
    PID=$(cat "${PID_FILE}")
    if kill -0 "${PID}" 2>/dev/null; then
      echo "Stopping dbsrt-relay-agent (PID: ${PID})..."
      kill -TERM "${PID}"
      for _ in $(seq 1 "${DEFAULT_SHUTDOWN_TIMEOUT_SEC}"); do
        if ! kill -0 "${PID}" 2>/dev/null; then
          break
        fi
        sleep 1
      done
      if kill -0 "${PID}" 2>/dev/null; then
        echo "Process did not terminate gracefully, sending SIGKILL..."
        kill -KILL "${PID}" || true
      fi
      echo "✓ Stopped dbsrt-relay-agent."
    fi
    rm -f "${PID_FILE}"
    ;;

  restart)
    "$0" stop
    "$0" start
    ;;

  status)
    echo "=== Local Relay Agent Status ==="
    if [[ -f "${PID_FILE}" ]]; then
      PID=$(cat "${PID_FILE}")
      if kill -0 "${PID}" 2>/dev/null; then
        echo "● dbsrt-relay-agent is running (PID: ${PID})"
        ps -o pid,user,%cpu,%mem,vsz,rss,comm -p "${PID}"
        echo ""
        echo "Local API Status (/v1/status):"
        curl -s "http://127.0.0.1:${PORT}/v1/status" | jq '{status, service, uptime_seconds, postgres_connected}' 2>/dev/null || echo "Unable to query /v1/status"
        echo ""
        echo "Remote Relay Metrics (/v1/metrics):"
        curl -s "http://127.0.0.1:${PORT}/v1/metrics" | jq '{total_requests, search_requests, remote_errors}' 2>/dev/null || true
      else
        echo "○ dbsrt-relay-agent is dead but PID file exists (${PID})"
      fi
    else
      echo "○ dbsrt-relay-agent is not running (no PID file)"
    fi
    ;;

  logs)
    if [[ -f "${LOG_FILE}" ]]; then
      tail -f "${LOG_FILE}"
    else
      echo "No log file found at ${LOG_FILE}."
    fi
    ;;

  demo)
    SAMPLE_PATH="${2:-$(resolve_sample_video)}"
    echo "=== Testing End-to-End Relay Search on: ${SAMPLE_PATH} ==="
    RESP=$(curl -s -f -X POST "http://127.0.0.1:${PORT}/v1/search" \
      -H "Content-Type: application/json" \
      -d '{
        "media_path": "'"${SAMPLE_PATH}"'",
        "search_mode": "phash",
        "language": "fr",
        "title": "Andor"
      }')
    echo "${RESP}" | jq '{media: .media_path, matched_media, confidence: .timing.confidence_percent, tier: .timing.matching_tier, candidates_count: (.candidates | length), top_candidate: .candidates[0].title}'
    ;;

  *)
    echo "Usage: $0 {start|stop|restart|status|logs|demo [media_path]}"
    exit 1
    ;;
esac
