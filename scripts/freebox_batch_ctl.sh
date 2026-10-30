#!/usr/bin/env bash
# Controls Freebox batch ingestion on o2switch, streaming from the RPi4 HTTP gateway
# through a Freebox NAT rule (WAN port -> RPi4:18089) restricted to the o2switch source IP
set -euo pipefail

# Named Operational Constants
DEFAULT_O2SWITCH_HOST="o2switch"
DEFAULT_REMOTE_BASE="dbsrt"
# o2switch only allows outbound TCP 21/80/443, and 21 is the Synology FTP NAT rule
DEFAULT_GATEWAY_WAN_PORT=80
DEFAULT_PROBE_PATH="/Disque_1/"
DEFAULT_WORKERS=5
DEFAULT_TARGETS_FILE="data/reports/freebox_batch_targets.json"

ACTION="${1:-status}"
O2SWITCH_HOST="${O2SWITCH_HOST:-${DEFAULT_O2SWITCH_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"
GATEWAY_WAN_PORT="${FREEBOX_GATEWAY_WAN_PORT:-${DEFAULT_GATEWAY_WAN_PORT}}"

# The Freebox public host is the Synology NAS_HOST (same box), read from o2switch etc/.nas_config
resolve_stream_base() {
  if [[ -n "${FREEBOX_STREAM_BASE:-}" ]]; then
    echo "${FREEBOX_STREAM_BASE%/}"
    return
  fi
  local host
  # shellcheck disable=SC2029
  host="$(ssh -o BatchMode=yes "${O2SWITCH_HOST}" "sed -n 's/^NAS_HOST=//p' ${REMOTE_BASE}/etc/.nas_config")"
  if [[ -z "${host}" ]]; then
    echo "❌ NAS_HOST missing in ${O2SWITCH_HOST}:${REMOTE_BASE}/etc/.nas_config (or set FREEBOX_STREAM_BASE)." >&2
    exit 1
  fi
  echo "http://${host}:${GATEWAY_WAN_PORT}"
}

# Any HTTP status (404 on a directory) proves o2switch -> NAT -> gateway works; 000 means unreachable
check_gateway() {
  local base="$1" code
  echo "=== Probing Freebox gateway from ${O2SWITCH_HOST} ==="
  # shellcheck disable=SC2029
  code="$(ssh -o BatchMode=yes "${O2SWITCH_HOST}" \
    "curl -sS -m 10 -o /dev/null -w '%{http_code}' '${base}${DEFAULT_PROBE_PATH}' 2>/dev/null || true")"
  if [[ ! "${code}" =~ ^[1-5][0-9][0-9]$ ]]; then
    echo "❌ Freebox gateway unreachable from ${O2SWITCH_HOST} (HTTP ${code:-000}): check the RPi4 gateway and the Freebox NAT rule."
    exit 1
  fi
  echo "✓ Gateway reachable (HTTP ${code})."
}

scan_and_sync_targets() {
  local limit_arg=()
  if [[ "${1:-}" == "--limit" && -n "${2:-}" ]]; then
    limit_arg=(--limit "$2")
  fi

  echo "=== Scanning eligible Freebox cinema & TV targets (excluding Photos/ and Backup/) ==="
  python3 scripts/scan_freebox_targets.py "${limit_arg[@]}"

  echo "=== Transferring targets catalog to ${O2SWITCH_HOST} ==="
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "mkdir -p ${REMOTE_BASE}/data/reports"
  rsync -avz "${DEFAULT_TARGETS_FILE}" "${O2SWITCH_HOST}:${REMOTE_BASE}/${DEFAULT_TARGETS_FILE}"
}

start_batch() {
  local stream_base
  stream_base="$(resolve_stream_base)"
  check_gateway "${stream_base}"

  local workers="${DEFAULT_WORKERS}"
  local limit=""
  shift || true
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --workers)
        workers="${2:?--workers requires count}"
        shift
        ;;
      --limit)
        limit="${2:?--limit requires count}"
        shift
        ;;
    esac
    shift
  done

  scan_and_sync_targets

  echo "=== Launching Freebox batch worker daemon on ${O2SWITCH_HOST} ==="
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "bash -s -- '${REMOTE_BASE}' '${stream_base}' '${DEFAULT_TARGETS_FILE}' '${workers}' '${limit}'" <<'REMOTE_BATCH'
BASE="$1"
STREAM_BASE="$2"
TARGETS_FILE="$3"
WORKERS="$4"
LIMIT="$5"

cd "${BASE}"
PID_FILE="run/batch_worker.pid"
LOG_FILE="logs/batch_worker.log"

if [[ -f "${PID_FILE}" ]]; then
  PID=$(cat "${PID_FILE}")
  if kill -0 "${PID}" 2>/dev/null; then
    echo "⚠️ Batch worker is already running with PID ${PID}."
    exit 0
  fi
fi

# Clean up leftover screen session
screen -X -S dbsrt_batch quit 2>/dev/null || true

# Lifecycle markers read by web monitor
echo "freebox" > run/batch_type
rm -f run/batch_stopped run/batch_aborted run/batch_queue.json

export PATH="$HOME/${BASE}/bin:$PATH"
export RAYON_NUM_THREADS=2

EXTRA_ARGS=()
if [[ -n "${LIMIT}" ]]; then
  EXTRA_ARGS+=(--limit "${LIMIT}")
fi

screen -dmS dbsrt_batch bash -c "echo \$\$ > '${PID_FILE}'; exec ./bin/python3 -u scripts/o2switch_batch_worker.py --stream-base '${STREAM_BASE}' --targets-file '${TARGETS_FILE}' --workers '${WORKERS}' --fresh-only ${EXTRA_ARGS[*]} >> '${LOG_FILE}' 2>&1"
sleep 1
if [[ -f "${PID_FILE}" ]]; then
  echo "✓ Started persistent background Freebox batch worker (PID: $(cat "${PID_FILE}"), concurrency: ${WORKERS} workers)."
else
  echo "❌ Failed to record PID file."
  exit 1
fi
REMOTE_BATCH
}

stop_batch() {
  echo "=== Stopping batch worker on ${O2SWITCH_HOST} ==="
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "bash -s -- '${REMOTE_BASE}'" <<'REMOTE_STOP'
cd "$1"
screen -X -S dbsrt_batch quit 2>/dev/null || true
if [[ -f run/batch_worker.pid ]]; then
  PID=$(cat run/batch_worker.pid)
  kill "${PID}" 2>/dev/null || true
  rm -f run/batch_worker.pid
fi
touch run/batch_stopped
echo "✓ Batch worker stopped."
REMOTE_STOP
}

status_batch() {
  echo "=== Freebox Batch Status on ${O2SWITCH_HOST} ==="
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "bash -s -- '${REMOTE_BASE}'" <<'REMOTE_STATUS'
cd "$1"
TYPE=$(cat run/batch_type 2>/dev/null || echo "unknown")
echo "BATCH_TYPE=${TYPE}"
if [[ -f run/batch_worker.pid ]] && kill -0 $(cat run/batch_worker.pid) 2>/dev/null; then
  echo "✓ Batch worker RUNNING (PID: $(cat run/batch_worker.pid))"
else
  echo "○ Batch worker NOT RUNNING"
fi

if [[ -f run/batch_queue.json ]]; then
  echo "--- Current Queue Progress ---"
  cat run/batch_queue.json
fi

echo "--- Last 10 Log Entries ---"
tail -n 10 logs/batch_worker.log 2>/dev/null || true
REMOTE_STATUS
}

case "${ACTION}" in
  start) start_batch "$@" ;;
  stop) stop_batch ;;
  status) status_batch ;;
  scan) scan_and_sync_targets "${@:2}" ;;
  test) check_gateway "$(resolve_stream_base)" ;;
  *)
    echo "Usage: $0 {start [--workers N] [--limit N]|stop|status|scan [--limit N]|test}"
    exit 1
    ;;
esac
