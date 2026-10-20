#!/usr/bin/env bash
# Controls deployment and lifecycle of dbsrt-server on Raspberry Pi 4
set -euo pipefail

ACTION="${1:-status}"
RPI_HOST="${RPI_HOST:-rpi4}"
IMAGE_NAME="${IMAGE_NAME:-dbsrt-server:rpi4}"
REMOTE_BASE="${REMOTE_BASE:-~/dbsrt}"
SERVER_PORT=8765

RPI_IP=$(ssh -G "${RPI_HOST}" 2>/dev/null | awk '/^hostname / {print $2}' || true)
if [[ -z "${RPI_IP}" ]]; then
  RPI_IP="${RPI_HOST}"
fi

case "${ACTION}" in
  push)
    echo "=== Streaming ARM64 image ${IMAGE_NAME} to ${RPI_HOST} ==="
    docker save "${IMAGE_NAME}" | ssh "${RPI_HOST}" "docker load"
    echo "✓ Image loaded successfully into ${RPI_HOST} docker daemon."
    ;;

  up)
    echo "=== Starting dbsrt-server on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose up -d"
    echo "Waiting for healthcheck on http://${RPI_IP}:${SERVER_PORT}/v1/health..."
    for _ in {1..30}; do
      if curl -s -f "http://${RPI_IP}:${SERVER_PORT}/v1/health" >/dev/null 2>&1; then
        echo "✓ Remote dbsrt-server is active and healthy on http://${RPI_IP}:${SERVER_PORT}"
        exit 0
      fi
      sleep 0.5
    done
    echo "⚠️ Healthcheck timed out. Checking remote container logs:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose logs --tail=20"
    exit 1
    ;;

  down)
    echo "=== Stopping dbsrt-server on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose down"
    echo "✓ Remote container stopped."
    ;;

  status)
    echo "=== Remote Container Status (${RPI_HOST}) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose ps" || true
    echo ""
    echo "=== Remote API Status (http://${RPI_IP}:${SERVER_PORT}/v1/status) ==="
    if curl -s -f "http://${RPI_IP}:${SERVER_PORT}/v1/status" 2>/dev/null | jq . 2>/dev/null; then
      :
    else
      echo "Remote API is not responding on http://${RPI_IP}:${SERVER_PORT}"
    fi
    ;;

  logs)
    ssh -t "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose logs -f"
    ;;

  demo)
    echo "=== Testing Remote Recognize API (200 bytes pHash) on http://${RPI_IP}:${SERVER_PORT} ==="
    curl -s -f -X POST "http://${RPI_IP}:${SERVER_PORT}/v1/recognize" \
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
    }' | jq .
    echo "✓ Remote API test successful!"
    ;;

  *)
    echo "Usage: $0 {push|up|down|status|logs|demo}"
    exit 1
    ;;
esac
