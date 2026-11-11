#!/usr/bin/env bash
# Builds dbsrt-server ARM64 Docker image with Bookworm glibc parity
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

IMAGE_NAME="${IMAGE_NAME:-dbsrt-server:rpi4}"
DOCKERFILE="${REPO_ROOT}/docker/Dockerfile.server"

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [--check | --dry-run | --help]"
  echo ""
  echo "Builds dbsrt-server ARM64 Docker image with Bookworm glibc parity."
  echo ""
  echo "Options:"
  echo "  --check, --dry-run  Verify Dockerfile presence and Docker daemon without building"
  echo "  --help, -h          Show this help message and exit"
  echo ""
  echo "Environment variables:"
  echo "  IMAGE_NAME          Docker tag name (default: dbsrt-server:rpi4)"
  exit 0
fi

if [[ ! -f "${DOCKERFILE}" ]]; then
  echo "❌ Error: Dockerfile not found at ${DOCKERFILE}" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "❌ Error: Docker CLI not found in PATH." >&2
  exit 1
fi

if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  echo "✓ REPO_ROOT  : ${REPO_ROOT}"
  echo "✓ Dockerfile : ${DOCKERFILE} (exists: yes)"
  echo "✓ Image tag  : ${IMAGE_NAME}"
  echo "✓ Docker CLI : $(command -v docker)"
  exit 0
fi

echo "=== Building dbsrt-server Docker image for ARM64 (${IMAGE_NAME}) ==="
docker build \
  --platform linux/arm64 \
  -f "${DOCKERFILE}" \
  -t "${IMAGE_NAME}" \
  "${REPO_ROOT}"

echo "✓ Successfully built ${IMAGE_NAME} (ARM64)"
