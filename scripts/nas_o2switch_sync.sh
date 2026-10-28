#!/usr/bin/env bash
# Synchronizes Synology NAS .phash fingerprints, compiled binary catalog,
# and SRT subtitles directly to Cloud o2switch hosting.
set -euo pipefail

# Named operational constants
DEFAULT_O2SWITCH_HOST="o2switch"
DEFAULT_REMOTE_BASE="dbsrt"
DEFAULT_RSYNC_BWLIMIT_KBPS=20000
DEFAULT_SCENE_CUT_THRESHOLD=16
DEFAULT_REMOTE_STATUS_URL="https://2026-pydbsrt.fr/dbsrt/v1/status"
DEFAULT_PASS_ENTRY="services/pydbsrt/o2switch_api_key"

O2SWITCH_HOST="${O2SWITCH_HOST:-${DEFAULT_O2SWITCH_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"
RSYNC_BWLIMIT="${RSYNC_BWLIMIT:-${DEFAULT_RSYNC_BWLIMIT_KBPS}}"
DRY_RUN=false

for arg in "$@"; do
  case "${arg}" in
    --dry-run | -n)
      DRY_RUN=true
      shift
      ;;
    --help | -h)
      echo "Usage: $0 [--dry-run]"
      echo "Synchronizes Synology NAS .phash fingerprints, compiled binary catalog,"
      echo "and subtitle store to Cloud o2switch under CloudLinux LVE bandwidth limits."
      exit 0
      ;;
    *) ;;
  esac
done

resolve_api_key() {
  if [[ -n "${DBSRT_REMOTE_API_KEY:-}" ]]; then
    echo "${DBSRT_REMOTE_API_KEY}"
    return 0
  fi
  if command -v pass >/dev/null 2>&1; then
    local pass_val
    pass_val=$(pass show "${DEFAULT_PASS_ENTRY}" 2>/dev/null || true)
    if [[ -n "${pass_val}" ]]; then
      echo "${pass_val}"
      return 0
    fi
  fi
  echo ""
}

resolve_synology_dirs() {
  local user_name="${USER:-$(whoami)}"
  RESOLVED_TVSHOW_DIR=""
  RESOLVED_VIDEO_DIR=""

  local tvshow_candidates=(
    "${SYNOLOGY_TVSHOW_DIR:-}"
    "/media/${user_name}/Synology/tvshow"
    "${HOME}/__SYNOLOGY__/tvshow"
    "/mnt/synology/tvshow"
  )
  for dir in "${tvshow_candidates[@]}"; do
    if [[ -n "${dir}" && -d "${dir}" ]]; then
      RESOLVED_TVSHOW_DIR="${dir}"
      break
    fi
  done

  local video_candidates=(
    "${SYNOLOGY_VIDEO_DIR:-}"
    "/media/${user_name}/Synology/video"
    "${HOME}/__SYNOLOGY__/video"
    "/mnt/synology/video"
  )
  for dir in "${video_candidates[@]}"; do
    if [[ -n "${dir}" && -d "${dir}" ]]; then
      RESOLVED_VIDEO_DIR="${dir}"
      break
    fi
  done
}

echo "=== DBS-RT SYNOLOGY -> O2SWITCH CLOUD SYNCHRONIZATION ==="
resolve_synology_dirs

if [[ -z "${RESOLVED_TVSHOW_DIR}" && -z "${RESOLVED_VIDEO_DIR}" ]]; then
  echo "❌ Error: No accessible Synology NAS mount found."
  echo "Checked candidates in /media/${USER:-synology}/Synology, \${HOME}/__SYNOLOGY__, /mnt/synology"
  exit 1
fi

echo "✓ Active Synology TVShow : ${RESOLVED_TVSHOW_DIR:-<none>}"
echo "✓ Active Synology Video  : ${RESOLVED_VIDEO_DIR:-<none>}"
echo "✓ Target Cloud Host      : ${O2SWITCH_HOST}"
echo "✓ Bandwidth Limit        : ${RSYNC_BWLIMIT} KB/s"
echo "✓ Dry-run Mode           : ${DRY_RUN}"

echo "--- 1. Checking SSH connectivity to ${O2SWITCH_HOST} ---"
if ! ssh -o BatchMode=yes -o ConnectTimeout=5 "${O2SWITCH_HOST}" true 2>/dev/null; then
  echo "❌ Error: Cannot connect to ${O2SWITCH_HOST} via SSH."
  exit 1
fi
echo "✓ SSH connection to ${O2SWITCH_HOST} verified."

PACK_EXTRA_ARGS=()
TVSHOW_PHASH="${RESOLVED_TVSHOW_DIR}/__PHASH__"
VIDEO_PHASH="${RESOLVED_VIDEO_DIR}/__PHASH__"

PRIMARY_PHASH_DIR=""
if [[ -d "${TVSHOW_PHASH}" ]]; then
  PRIMARY_PHASH_DIR="${TVSHOW_PHASH}"
  if [[ -d "${VIDEO_PHASH}" ]]; then
    PACK_EXTRA_ARGS+=(--extra-dir "${VIDEO_PHASH}")
  fi
elif [[ -d "${VIDEO_PHASH}" ]]; then
  PRIMARY_PHASH_DIR="${VIDEO_PHASH}"
else
  echo "⚠️ Warning: No __PHASH__ directory found on Synology mounts."
  PRIMARY_PHASH_DIR="data/__PHASH__"
fi

OUTPUT_DBPHASH="target/catalog_nas.dbphash"
mkdir -p target

echo "--- 2. Compiling unified Multi-Index Hash (.dbphash) from NAS fingerprints ---"
if [[ ! -f "target/release/dbsrt" ]]; then
  echo "Building release binary dbsrt..."
  cargo build --release --bin dbsrt
fi

./target/release/dbsrt pack \
  --phash-dir "${PRIMARY_PHASH_DIR}" \
  "${PACK_EXTRA_ARGS[@]}" \
  --output "${OUTPUT_DBPHASH}" \
  --threshold "${DEFAULT_SCENE_CUT_THRESHOLD}"

DBPHASH_SIZE=$(du -h "${OUTPUT_DBPHASH}" | cut -f1)
echo "✓ Binary index compiled: ${OUTPUT_DBPHASH} (${DBPHASH_SIZE})"

echo "--- 3. Transferring catalog to o2switch (${O2SWITCH_HOST}) ---"
RSYNC_FLAGS=(-avz --bwlimit="${RSYNC_BWLIMIT}")
if [[ "${DRY_RUN}" == "true" ]]; then
  RSYNC_FLAGS+=(--dry-run)
fi

rsync "${RSYNC_FLAGS[@]}" "${OUTPUT_DBPHASH}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/catalog.dbphash"
echo "✓ Catalog synchronized to ${O2SWITCH_HOST}:${REMOTE_BASE}/data/catalog.dbphash"

echo "--- 4. Synchronizing local SRT catalog to o2switch ---"
if [[ -d "data/SRT" ]]; then
  rsync "${RSYNC_FLAGS[@]}" "data/SRT/" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/SRT/"
  echo "✓ SRT catalog synchronized to ${O2SWITCH_HOST}:${REMOTE_BASE}/data/SRT/"
fi

if [[ "${DRY_RUN}" == "true" ]]; then
  echo "=== Dry-run completed successfully (no daemon restart) ==="
  exit 0
fi

echo "--- 5. Hot-restarting dbsrt-server on o2switch ---"
./scripts/o2switch_ctl.sh restart

echo "--- 6. Verifying Remote Status & Ingested Catalog ---"
API_KEY=$(resolve_api_key)
if [[ -n "${API_KEY}" ]]; then
  STATUS_RESP=$(curl -s -f -m 5 "${DEFAULT_REMOTE_STATUS_URL}?api_key=${API_KEY}" 2>/dev/null || echo "")
  if [[ -n "${STATUS_RESP}" ]]; then
    echo "✓ Remote Cloud status: ${STATUS_RESP}"
  else
    echo "⚠️ Remote status did not respond in 5s. Daemon may be warming up."
  fi
else
  echo "⚠️ Notice: No API key found in pass store (${DEFAULT_PASS_ENTRY})."
fi

echo "=== Synchronization complete! ==="
