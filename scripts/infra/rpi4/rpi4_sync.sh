#!/usr/bin/env bash
# Synchronizes catalogue and subtitles to Raspberry Pi 4
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

RPI_HOST="${RPI_HOST:-rpi4}"
REMOTE_BASE="${REMOTE_BASE:-~/dbsrt}"
DRY_RUN=false
CHECK_ONLY=false

for arg in "$@"; do
  case "${arg}" in
    --dry-run | -n)
      DRY_RUN=true
      ;;
    --check)
      CHECK_ONLY=true
      ;;
    --help | -h)
      echo "Usage: $(basename "$0") [--dry-run] [--check] [--help]"
      echo ""
      echo "Synchronizes catalogue, subtitles, docker-compose and PG17 assets to RPi4."
      echo ""
      echo "Options:"
      echo "  --dry-run, -n  Run rsync in dry-run mode without modifying remote files"
      echo "  --check        Inspect local assets and SSH connectivity without transfer"
      echo "  --help, -h     Show this help message and exit"
      echo ""
      echo "Environment variables:"
      echo "  RPI_HOST       Target SSH host for Raspberry Pi 4 (default: rpi4)"
      echo "  REMOTE_BASE    Remote directory path (default: ~/dbsrt)"
      exit 0
      ;;
    *) ;;
  esac
done

if [[ "${CHECK_ONLY}" == "true" ]]; then
  echo "✓ REPO_ROOT     : ${REPO_ROOT}"
  echo "✓ RPI_HOST      : ${RPI_HOST}"
  echo "✓ REMOTE_BASE   : ${REMOTE_BASE}"
  echo "✓ Catalog file  : $(test -f "${REPO_ROOT}/data/catalog.dbphash" && echo "present" || echo "missing")"
  echo "✓ SRT dir       : $(test -d "${REPO_ROOT}/data/SRT" && echo "present" || echo "missing")"
  echo "✓ pHash dir     : $(test -d "${REPO_ROOT}/data/__PHASH__" && echo "present" || echo "missing")"
  echo "✓ Compose file  : $(test -f "${REPO_ROOT}/docker/docker-compose.rpi4.yml" && echo "present" || echo "missing")"
  echo "✓ SSH reachable : $(ssh -o BatchMode=yes -o ConnectTimeout=5 "${RPI_HOST}" true 2>/dev/null && echo "yes" || echo "no")"
  echo "=== Check mode completed successfully ==="
  exit 0
fi

echo "=== Synchronizing pydbsrt catalog and subtitles to ${RPI_HOST}:${REMOTE_BASE} ==="

RSYNC_OPTS=(-avz --progress)
if [[ "${DRY_RUN}" == "true" ]]; then
  RSYNC_OPTS+=(--dry-run)
  echo "[Dry-Run Mode: No files will be modified on remote]"
fi

# shellcheck disable=SC2029
if [[ "${DRY_RUN}" != "true" ]]; then
  ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BASE}/data"
fi

if [[ -f "${REPO_ROOT}/data/catalog.dbphash" ]]; then
  echo "Syncing catalog.dbphash..."
  rsync "${RSYNC_OPTS[@]}" "${REPO_ROOT}/data/catalog.dbphash" "${RPI_HOST}:${REMOTE_BASE}/data/"
else
  echo "⚠️ Warning: ${REPO_ROOT}/data/catalog.dbphash not found locally, skipping."
fi

if [[ -d "${REPO_ROOT}/data/SRT" ]]; then
  echo "Syncing data/SRT..."
  rsync "${RSYNC_OPTS[@]}" "${REPO_ROOT}/data/SRT" "${RPI_HOST}:${REMOTE_BASE}/data/"
fi

if [[ -d "${REPO_ROOT}/data/__PHASH__" ]]; then
  echo "Syncing data/__PHASH__..."
  rsync "${RSYNC_OPTS[@]}" "${REPO_ROOT}/data/__PHASH__" "${RPI_HOST}:${REMOTE_BASE}/data/"
fi

echo "Syncing docker-compose.rpi4.yml..."
if [[ "${DRY_RUN}" != "true" ]]; then
  scp "${REPO_ROOT}/docker/docker-compose.rpi4.yml" "${RPI_HOST}:${REMOTE_BASE}/docker-compose.yml"
else
  echo "(dry-run) would scp ${REPO_ROOT}/docker/docker-compose.rpi4.yml to ${RPI_HOST}:${REMOTE_BASE}/docker-compose.yml"
fi

echo "Syncing PostgreSQL 17 Dockerfile & scripts..."
# shellcheck disable=SC2029
if [[ "${DRY_RUN}" != "true" ]]; then
  ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BASE}/docker/pg17"
fi
rsync "${RSYNC_OPTS[@]}" "${REPO_ROOT}/docker/pg-spgist_hamming/17-develop/" "${RPI_HOST}:${REMOTE_BASE}/docker/pg17/"

echo "✓ Synchronization complete."
