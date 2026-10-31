#!/usr/bin/env bash
# Controls deployment and lifecycle of standalone dbsrt-server on Raspberry Pi 4 (systemd user service)
set -euo pipefail

# Named operational constants
DEFAULT_SERVER_PORT=8765
DEFAULT_HEALTHCHECK_MAX_RETRIES=20
DEFAULT_HEALTHCHECK_SLEEP_SEC=0.5
DEFAULT_DEMO_SAMPLE_LINES=8

ACTION="${1:-status}"
RPI_HOST="${RPI_HOST:-rpi4}"
SERVER_PORT="${SERVER_PORT:-${DEFAULT_SERVER_PORT}}"
REMOTE_BIN_DIR="${REMOTE_BIN_DIR:-~/.local/bin}"
REMOTE_SYSTEMD_DIR="${REMOTE_SYSTEMD_DIR:-~/.config/systemd/user}"
SERVICE_NAME="dbsrt-server.service"
TARGET_DIR="target/aarch64-unknown-linux-gnu/release"
SERVER_BINARY="${TARGET_DIR}/dbsrt-server"

RPI_IP=$(ssh -G "${RPI_HOST}" 2>/dev/null | awk '/^hostname / {print $2}' || true)
if [[ -z "${RPI_IP}" ]]; then
  RPI_IP="${RPI_HOST}"
fi

get_effective_port() {
  local eff_port
  # shellcheck disable=SC2029
  eff_port=$(ssh "${RPI_HOST}" "jq -r .port /tmp/dbsrt-server.json 2>/dev/null" || true)
  if [[ -n "${eff_port}" && "${eff_port}" != "null" ]]; then
    echo "${eff_port}"
  else
    echo "${SERVER_PORT}"
  fi
}

case "${ACTION}" in
  deploy)
    if [[ ! -f "${SERVER_BINARY}" ]]; then
      echo "Binary not found at ${SERVER_BINARY}. Triggering ARM64 build..."
      ./scripts/rpi4_cli.sh build
    fi
    echo "=== Deploying standalone dbsrt-server to ${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt-server ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BIN_DIR} ${REMOTE_SYSTEMD_DIR}"
    scp "${SERVER_BINARY}" "${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt-server"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "chmod +x ${REMOTE_BIN_DIR}/dbsrt-server"

    echo "=== Configuring systemd user service on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p %h/dbsrt/run/empty_phash"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cat <<'EOF' > ${REMOTE_SYSTEMD_DIR}/${SERVICE_NAME}
[Unit]
Description=dbsrt Standalone Zero-DB Server
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/dbsrt-server --host 0.0.0.0 --port ${SERVER_PORT} --dbphash %h/dbsrt/data/catalog.dbphash --subtitles-dir %h/dbsrt/data/SRT --phash-dir %h/dbsrt/run/empty_phash
Restart=always
RestartSec=3
KillMode=process
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "systemctl --user daemon-reload && systemctl --user enable ${SERVICE_NAME}"
    echo "✓ Standalone dbsrt-server deployed and enabled on ${RPI_HOST}."
    ;;

  start)
    echo "=== Starting standalone dbsrt-server on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "systemctl --user start ${SERVICE_NAME}"
    EFF_PORT=$(get_effective_port)
    echo "Waiting for healthcheck on http://${RPI_IP}:${EFF_PORT}/v1/health..."
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      EFF_PORT=$(get_effective_port)
      if curl -s -f "http://${RPI_IP}:${EFF_PORT}/v1/health" >/dev/null 2>&1; then
        echo "✓ Standalone dbsrt-server is active and healthy on http://${RPI_IP}:${EFF_PORT}"
        exit 0
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done
    echo "⚠️ Healthcheck timed out. Checking journal logs:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "journalctl --user -u ${SERVICE_NAME} -n 20 --no-pager"
    exit 1
    ;;

  stop)
    echo "=== Stopping standalone dbsrt-server on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "systemctl --user stop ${SERVICE_NAME}"
    echo "✓ Standalone dbsrt-server stopped."
    ;;

  restart)
    echo "=== Restarting standalone dbsrt-server on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "systemctl --user restart ${SERVICE_NAME}"
    echo "✓ Standalone dbsrt-server restarted."
    ;;

  status)
    echo "=== Standalone Systemd User Service Status (${RPI_HOST}) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "systemctl --user status ${SERVICE_NAME} --no-pager" || true
    EFF_PORT=$(get_effective_port)
    echo ""
    echo "=== Remote API Status (http://${RPI_IP}:${EFF_PORT}/v1/status) ==="
    if curl -s -f "http://${RPI_IP}:${EFF_PORT}/v1/status" 2>/dev/null | jq . 2>/dev/null; then
      :
    else
      echo "Remote API is not responding on http://${RPI_IP}:${EFF_PORT}"
    fi
    ;;

  logs)
    ssh -t "${RPI_HOST}" "journalctl --user -u ${SERVICE_NAME} -f"
    ;;

  demo)
    EFF_PORT=$(get_effective_port)
    echo "=== Testing Remote Recognize API on http://${RPI_IP}:${EFF_PORT} ==="
    RESP=$(curl -s -f -X POST "http://${RPI_IP}:${EFF_PORT}/v1/recognize" \
      -H "Content-Type: application/json" \
      -d '{
      "query_fingerprint": [
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b"
      ],
      "fps": 25.0,
      "language": "fr"
    }')
    echo "${RESP}" | jq .
    CANDIDATE_ID=$(echo "${RESP}" | jq -r '.candidates[0].candidate_id // empty')
    if [[ "${CANDIDATE_ID}" =~ ^sub:sha256:(.+) ]]; then
      HASH="${BASH_REMATCH[1]}"
      echo ""
      echo "=== Testing Remote CAS Subtitle Fetch (/v1/subtitles/${HASH}) ==="
      SRT_CONTENT=$(curl -s -f "http://${RPI_IP}:${EFF_PORT}/v1/subtitles/${HASH}")
      (echo "${SRT_CONTENT}" | head -n "${DEFAULT_DEMO_SAMPLE_LINES}") || true
      echo "✓ Remote CAS Subtitle Download verified!"
    fi
    echo "✓ Remote API test successful!"
    ;;

  *)
    echo "Usage: $0 {deploy|start|stop|restart|status|logs|demo}"
    exit 1
    ;;
esac
