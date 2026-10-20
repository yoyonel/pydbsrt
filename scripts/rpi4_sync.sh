#!/usr/bin/env bash
# Synchronizes catalogue and subtitles to Raspberry Pi 4
set -euo pipefail

RPI_HOST="${RPI_HOST:-rpi4}"
REMOTE_BASE="${REMOTE_BASE:-~/dbsrt}"

echo "=== Synchronizing pydbsrt catalog and subtitles to ${RPI_HOST}:${REMOTE_BASE} ==="

# shellcheck disable=SC2029
ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BASE}/data"

if [[ -f "data/catalog.dbphash" ]]; then
  echo "Syncing catalog.dbphash..."
  rsync -avz --progress "data/catalog.dbphash" "${RPI_HOST}:${REMOTE_BASE}/data/"
else
  echo "⚠️ Warning: data/catalog.dbphash not found locally, skipping."
fi

if [[ -d "data/SRT" ]]; then
  echo "Syncing data/SRT..."
  rsync -avz --progress "data/SRT" "${RPI_HOST}:${REMOTE_BASE}/data/"
fi

echo "Syncing docker-compose.rpi4.yml..."
scp "docker/docker-compose.rpi4.yml" "${RPI_HOST}:${REMOTE_BASE}/docker-compose.yml"

echo "✓ Synchronization complete."
