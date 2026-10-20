#!/usr/bin/env bash
# Builds dbsrt-server ARM64 Docker image with Bookworm glibc parity
set -euo pipefail

IMAGE_NAME="${IMAGE_NAME:-dbsrt-server:rpi4}"

echo "=== Building dbsrt-server Docker image for ARM64 (${IMAGE_NAME}) ==="
docker build \
  --platform linux/arm64 \
  -f docker/Dockerfile.server \
  -t "${IMAGE_NAME}" .

echo "✓ Successfully built ${IMAGE_NAME} (ARM64)"
