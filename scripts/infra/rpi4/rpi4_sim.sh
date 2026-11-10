#!/usr/bin/env bash
# Manages local ARM64 simulation of dbsrt-server on x86 host via QEMU
set -euo pipefail

ACTION="${1:-status}"
COMPOSE_FILE="sim/docker-compose.sim.yml"
SIM_PORT=8766

case "${ACTION}" in
  up)
    HOST_ARCH="$(uname -m)"
    if [[ "${HOST_ARCH}" != "aarch64" && "${HOST_ARCH}" != "arm64" ]]; then
      if [[ ! -f "/proc/sys/fs/binfmt_misc/qemu-aarch64" ]]; then
        echo "❌ Error: QEMU ARM64 binfmt emulator is not registered on host kernel."
        echo "💡 To enable ARM64 container emulation on this ${HOST_ARCH} host, run:"
        echo "   docker run --privileged --rm tonistiigi/binfmt --install arm64"
        exit 1
      fi
    fi
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
    echo "=== [1/6] Testing ARM64 simulated container via /v1/health & /v1/status ==="
    curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/health" | jq .
    echo ""
    curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/status" | jq .
    echo ""

    echo "=== [2/6] Testing /v1/metrics ==="
    curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/metrics" | jq .
    echo ""

    echo "=== [3/6] Testing /studio Web UI (HTTP 200) ==="
    curl -s -f -o /dev/null -w "HTTP %{http_code} OK (bytes: %{size_download})\n" "http://127.0.0.1:${SIM_PORT}/studio"
    echo ""

    echo "=== [4/6] Testing /v1/recognize (Multi-Index Hashing RAM on ARM64) ==="
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
    echo ""

    echo "=== [5/6] Testing /v1/search (Full Search API) ==="
    curl -s -f -X POST "http://127.0.0.1:${SIM_PORT}/v1/search" \
      -H "Content-Type: application/json" \
      -d '{
      "media_path": "/app/tests/data/big_buck_bunny_trailer_480p.mp4",
      "language": "en",
      "search_mode": "catalog"
    }' | jq .
    echo ""

    echo "=== [6/6] Testing /v1/subtitles/:hash (Content-Addressable Storage CAS) ==="
    if [[ ! -f data/SRT/couplings.json ]]; then
      echo "⚠ data/SRT/couplings.json not found, skipping CAS test. Run 'bash scripts/init_couplings_seed.sh' first."
    else
      SUB_HASH=$(grep -m 1 "subtitles_hash" data/SRT/couplings.json | tr -d '", ' | cut -d: -f2)
      if [[ -n "${SUB_HASH}" ]]; then
        HEAD_SRT=$(curl -s -f "http://127.0.0.1:${SIM_PORT}/v1/subtitles/${SUB_HASH}" | head -n 4)
        echo "✓ CAS successfully retrieved subtitle track for hash ${SUB_HASH}:"
        echo "${HEAD_SRT}"
      fi
    fi
    echo ""
    echo "✓ All ARM64 simulated RPi4 smoke tests completed successfully!"
    ;;

  *)
    echo "Usage: $0 {up|down|status|logs|test}"
    exit 1
    ;;
esac
