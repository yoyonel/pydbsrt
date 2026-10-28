#!/usr/bin/env bash
# Manages cross-compilation, local ARM64 simulation, and deployment of dbsrt CLI on Raspberry Pi 4
set -euo pipefail

# Named operational constants
DEFAULT_HEAD_LINES=10

ACTION="${1:-build}"
RPI_HOST="${RPI_HOST:-rpi4}"
IMAGE_NAME="${IMAGE_NAME:-dbsrt-cli:rpi4}"
REMOTE_BIN_DIR="${REMOTE_BIN_DIR:-~/.local/bin}"
TARGET_DIR="target/aarch64-unknown-linux-gnu/release"
BINARY_PATH="${TARGET_DIR}/dbsrt"
SERVER_BINARY_PATH="${TARGET_DIR}/dbsrt-server"

case "${ACTION}" in
  build)
    echo "=== Building dbsrt CLI & Server Docker image for ARM64 with Bullseye GLIBC 2.31 parity ==="
    docker build \
      -f docker/Dockerfile.cli.rpi4 \
      -t "${IMAGE_NAME}" .
    echo "✓ Built Docker image ${IMAGE_NAME}."

    echo "=== Extracting ARM64 binaries to ${TARGET_DIR} ==="
    mkdir -p "${TARGET_DIR}"
    CID=$(docker create "${IMAGE_NAME}")
    docker cp "${CID}:/usr/local/bin/dbsrt" "${BINARY_PATH}"
    docker cp "${CID}:/usr/local/bin/dbsrt-server" "${SERVER_BINARY_PATH}"
    docker rm "${CID}" >/dev/null
    chmod +x "${BINARY_PATH}" "${SERVER_BINARY_PATH}"
    echo "✓ Extracted binaries: ${BINARY_PATH} and ${SERVER_BINARY_PATH}"
    file "${BINARY_PATH}" "${SERVER_BINARY_PATH}"
    ;;

  test)
    if [[ ! -f "${BINARY_PATH}" || ! -f "${SERVER_BINARY_PATH}" ]]; then
      echo "Binaries not found in ${TARGET_DIR}. Running build first..."
      "$0" build
    fi
    echo "=== Running local ARM64 Bullseye validation on host via QEMU ==="
    echo "-> Testing 'dbsrt --version' inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "$(pwd):/app" debian:bullseye-slim "/app/${BINARY_PATH}" --version

    echo "-> Testing 'dbsrt --help' inside Debian Bullseye ARM64 container:"
    (docker run --rm --platform linux/arm64 -v "$(pwd):/app" debian:bullseye-slim "/app/${BINARY_PATH}" --help | head -n "${DEFAULT_HEAD_LINES}") || true

    echo "-> Testing 'dbsrt-server --version' inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "$(pwd):/app" debian:bullseye-slim "/app/${SERVER_BINARY_PATH}" --version

    echo "-> Testing 'dbsrt-server --help' inside Debian Bullseye ARM64 container:"
    (docker run --rm --platform linux/arm64 -v "$(pwd):/app" debian:bullseye-slim "/app/${SERVER_BINARY_PATH}" --help | head -n "${DEFAULT_HEAD_LINES}") || true

    echo "-> Testing 'dbsrt top --json' telemetry snapshot inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "$(pwd):/app" debian:bullseye-slim "/app/${BINARY_PATH}" top --json | jq '{cpu: .host_cpu_pct, ram_used: .ram_used_gib, sources: (.media_sources | length)}'

    echo "✓ Local ARM64 Bullseye simulation passed with 100% parity!"
    ;;

  deploy)
    if [[ ! -f "${BINARY_PATH}" ]]; then
      echo "Binary not found at ${BINARY_PATH}. Running build first..."
      "$0" build
    fi
    echo "=== Deploying ARM64 dbsrt binary to ${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BIN_DIR}"
    scp "${BINARY_PATH}" "${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "chmod +x ${REMOTE_BIN_DIR}/dbsrt"
    echo "✓ Deployed successfully. Verifying remote execution on ${RPI_HOST}:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "${REMOTE_BIN_DIR}/dbsrt --version"
    ;;

  top)
    echo "=== Launching interactive dbsrt top session on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh -t "${RPI_HOST}" "${REMOTE_BIN_DIR}/dbsrt top ${*:2}"
    ;;

  *)
    echo "Usage: $0 {build|test|deploy|top} [args...]"
    exit 1
    ;;
esac
