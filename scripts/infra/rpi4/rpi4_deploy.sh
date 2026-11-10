#!/usr/bin/env bash
# Controls deployment and lifecycle of dbsrt-server on Raspberry Pi 4
set -euo pipefail

ACTION="${1:-status}"
RPI_HOST="${RPI_HOST:-rpi4}"
IMAGE_NAME="${IMAGE_NAME:-dbsrt-server:rpi4}"
REMOTE_BASE="${REMOTE_BASE:-~/dbsrt}"
# Operational & networking constants
SERVER_PORT=8765
RPI_DB_PORT="${PSQL_PORT:-5433}"
DEFAULT_HEALTHCHECK_MAX_RETRIES=30
DEFAULT_HEALTHCHECK_SLEEP_SEC=0.5
LOG_TAIL_TIMEOUT_LINES=20
DEMO_SAMPLE_LINES=8

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
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      if curl -s -f "http://${RPI_IP}:${SERVER_PORT}/v1/health" >/dev/null 2>&1; then
        echo "✓ Remote dbsrt-server is active and healthy on http://${RPI_IP}:${SERVER_PORT}"
        exit 0
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done
    echo "⚠️ Healthcheck timed out. Checking remote container logs:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose logs --tail=${LOG_TAIL_TIMEOUT_LINES}"
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
    RESP=$(curl -s -f -X POST "http://${RPI_IP}:${SERVER_PORT}/v1/recognize" \
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
      SRT_CONTENT=$(curl -s -f "http://${RPI_IP}:${SERVER_PORT}/v1/subtitles/${HASH}")
      echo "${SRT_CONTENT}" | head -n "${DEMO_SAMPLE_LINES}"
      echo "✓ Remote CAS Subtitle Download verified!"
    fi
    echo "✓ Remote API test successful!"
    ;;

  db-build)
    echo "=== Building PostgreSQL 17 BK-Tree image natively on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BASE}/docker/pg17"
    scp docker/pg-spgist_hamming/17-develop/* "${RPI_HOST}:${REMOTE_BASE}/docker/pg17/"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE}/docker/pg17 && docker build -t pg-spgist_hamming:17-rpi4 ."
    echo "✓ Built pg-spgist_hamming:17-rpi4 successfully on ${RPI_HOST}."
    ;;

  db-up)
    echo "=== Starting dedicated PostgreSQL 17 container on ${RPI_HOST} (port ${RPI_DB_PORT}) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose up -d db"
    echo "Waiting for PostgreSQL 17 to become ready on ${RPI_IP}:${RPI_DB_PORT}..."
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      if ssh "${RPI_HOST}" "docker exec dbsrt-pg17 pg_isready -U docker -d bktreedb" >/dev/null 2>&1; then
        echo "✓ Remote PostgreSQL 17 is active and ready on ${RPI_IP}:${RPI_DB_PORT}"
        exit 0
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done
    echo "⚠️ PostgreSQL readiness timed out. Container logs:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "docker logs --tail=${LOG_TAIL_TIMEOUT_LINES} dbsrt-pg17"
    exit 1
    ;;

  db-down)
    echo "=== Stopping dedicated PostgreSQL 17 container on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose stop db"
    echo "✓ Dedicated PostgreSQL 17 container stopped (legacy DB untouched)."
    ;;

  db-logs)
    ssh -t "${RPI_HOST}" "cd ${REMOTE_BASE} && docker compose logs -f db"
    ;;

  db-status)
    echo "=== Remote PostgreSQL 17 Container Status (${RPI_HOST}) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "docker ps --filter name=dbsrt-pg17 --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'" || true
    echo ""
    echo "=== Remote PostgreSQL Migration Status (${RPI_HOST} -> ${RPI_IP}:${RPI_DB_PORT}) ==="
    ./target/release/dbsrt migrate --status --host "${RPI_IP}" --port "${RPI_DB_PORT}" "${@:2}"
    ;;

  db-migrate)
    echo "=== Remote PostgreSQL 17 Container Status (${RPI_HOST}) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "docker ps --filter name=dbsrt-pg17 --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'" || true
    echo ""
    echo "=== Running PostgreSQL Migrations on ${RPI_HOST} (${RPI_IP}:${RPI_DB_PORT}) ==="
    ./target/release/dbsrt migrate --host "${RPI_IP}" --port "${RPI_DB_PORT}" "${@:2}"
    ;;

  *)
    echo "Usage: $0 {push|up|down|status|logs|demo|db-build|db-up|db-down|db-logs|db-status|db-migrate}"
    exit 1
    ;;
esac
