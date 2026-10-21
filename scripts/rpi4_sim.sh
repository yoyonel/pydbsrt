#!/usr/bin/env bash
# Manages local ARM64 simulation of dbsrt-server on x86 host via QEMU
set -euo pipefail

ACTION="${1:-status}"
COMPOSE_FILE="sim/docker-compose.sim.yml"
SIM_PORT=8766

case "${ACTION}" in
  up)
    echo "Starting ARM64 simulation container on http://127.0.0.1:${SIM_PORT}..."
    docker compose -f "${COMPOSE_FILE}" up -d
    echo "Waiting for healthcheck..."
    for _ in {1..30}; do
      if curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/health" >/dev/null 2>&1; then
        echo "✓ Simulation container healthy and active on port ${SIM_PORT}"
        exit 0
      fi
      sleep 0.5
    done
    echo "⚠️ Healthcheck timed out. Recent container logs:"
    docker compose -f "${COMPOSE_FILE}" logs --tail=20
    exit 1
    ;;

  down)
    echo "Stopping ARM64 simulation container..."
    docker compose -f "${COMPOSE_FILE}" down
    echo "✓ Simulation container stopped."
    ;;

  status)
    docker compose -f "${COMPOSE_FILE}" ps
    echo ""
    if curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/status" 2>/dev/null | jq . 2>/dev/null; then
      :
    else
      echo "Simulation container is not responding on http://127.0.0.1:${SIM_PORT}"
    fi
    ;;

  logs)
    docker compose -f "${COMPOSE_FILE}" logs -f --tail=50
    ;;

  test)
    echo "=== Testing ARM64 simulated container via /v1/health & /v1/status ==="
    curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/health"
    echo ""
    echo "=== Testing /v1/metrics ==="
    curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/metrics"
    echo ""
    echo "=== Testing /v1/recognize (200 bytes pHash payload) ==="
    curl -s -f -X POST "http://127.0.0.1:${SIM_PORT}/v1/recognize" \
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
    echo "✓ Test completed successfully!"
    ;;

  *)
    echo "Usage: $0 {up|down|status|logs|test}"
    exit 1
    ;;
esac
