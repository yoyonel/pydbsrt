#!/usr/bin/env bash
# Remote-side lifecycle of dbsrt-server on o2switch: start | stop | status | pids | watchdog.
# Run by scripts/o2switch_ctl.sh over SSH and by the cron watchdog (deployed to ~/dbsrt/bin/).
# Three starters exist (PHP gateway, cron watchdog, start): all spawn the same absolute command
# line, so a daemon is any process matching "bin/dbsrt-server --unix-socket <socket>".
# usage: dbsrt_server_ctl.sh <action> [base_dir] [socket]   (relative paths resolve from $HOME)
set -euo pipefail

ACTION="${1:-status}"
cd "${HOME}"
abs() { [[ "$1" == /* ]] && printf '%s' "$1" || printf '%s/%s' "${HOME}" "$1"; }
BASE="$(abs "${2:-dbsrt}")"
SOCKET="$(abs "${3:-${BASE}/run/dbsrt-server.sock}")"
BIN="${BASE}/bin/dbsrt-server"
DATA_DIR="${BASE}/data"
LOG_FILE="${BASE}/logs/dbsrt-server.log"
# Shared with the PHP gateway spawn_daemon(): serializes every spawn and the stop marker
SPAWN_LOCK="${BASE}/run/gateway_spawn.lock"
# Set by stop, cleared by start: while present the gateway answers 503 and the watchdog skips
STOP_MARKER="${BASE}/run/dbsrt-server.stopped"
# Written by earlier versions of start; removed by stop
LEGACY_PID_FILE="${BASE}/run/dbsrt-server.pid"
SHUTDOWN_TIMEOUT_SEC="${DBSRT_SHUTDOWN_TIMEOUT_SEC:-10}"
# The gateway holds the lock ~4 s at most (health probe + spawn retries)
LOCK_WAIT_SEC="${DBSRT_LOCK_WAIT_SEC:-30}"
PATTERN="bin/dbsrt-server --unix-socket ${SOCKET}( |$)"

daemon_pids() { pgrep -u "$(id -u)" -f -- "${PATTERN}" || true; }

# Takes SPAWN_LOCK on fd 9 with a bounded wait: a leaked holder (a daemon spawned by a gateway
# request killed before its unlock) must fail loudly, not hang stop or pile up cron runs
lock_or_fail() {
  flock -w "${LOCK_WAIT_SEC}" 9 || {
    echo "❌ spawn lock ${SPAWN_LOCK} still held after ${LOCK_WAIT_SEC}s."
    exit 1
  }
}

# Caller holds SPAWN_LOCK on fd 9: the daemon must not inherit it, or it would hold the lock
# for its whole life and block every later spawn and stop
spawn() {
  mkdir -p "${BASE}/run/empty_phash" "${BASE}/logs"
  # Background a simple command: a backgrounded `cd && ...` list keeps a bash fork alive that
  # holds the caller's stdout (an SSH session or a test pipe would never close)
  (
    cd "${BASE}" || exit 1
    setsid "${BIN}" --unix-socket "${SOCKET}" \
      --dbphash "${DATA_DIR}/catalog.dbphash" \
      --subtitles-dir "${DATA_DIR}/SRT" \
      --phash-dir "${BASE}/run/empty_phash" \
      --api-key-file "${BASE}/etc/api_key.secret" \
      </dev/null >>"${LOG_FILE}" 2>&1 9>&- &
  )
}

case "${ACTION}" in
  start)
    mkdir -p "${BASE}/run"
    if [[ -n "$(daemon_pids)" ]]; then
      rm -f "${STOP_MARKER}"
      echo "⚠️ dbsrt-server is already running (PID: $(daemon_pids | paste -sd ' '))."
      exit 0
    fi
    # Loose .phash files are mmapped with an in-RAM MIH each (~6x disk size), which blows the
    # LVE memory cap; pack them into the catalog and serve it with an empty loose directory.
    if "${BASE}/bin/dbsrt" pack -p "${DATA_DIR}/__PHASH__" -o "${BASE}/run/catalog.pack.tmp" >/dev/null 2>&1; then
      mv -f "${BASE}/run/catalog.pack.tmp" "${DATA_DIR}/catalog.dbphash"
    else
      echo "⚠️ Catalog pack failed; serving the previous catalog."
    fi
    (
      lock_or_fail
      rm -f "${STOP_MARKER}"
      [[ -n "$(daemon_pids)" ]] || spawn
    ) 9>"${SPAWN_LOCK}"
    sleep 1
    PIDS="$(daemon_pids | paste -sd ' ')"
    if [[ -z "${PIDS}" ]]; then
      echo "❌ dbsrt-server exited right after spawn. Tail of ${LOG_FILE}:"
      tail -n 20 "${LOG_FILE}" || true
      exit 1
    fi
    echo "✓ Started dbsrt-server (PID: ${PIDS})."
    ;;

  stop)
    mkdir -p "${BASE}/run"
    # Marker under the spawn lock: no gateway or cron spawn can start once it is set
    if ! (
      lock_or_fail
      touch "${STOP_MARKER}"
    ) 9>"${SPAWN_LOCK}"; then
      # The holder may be a daemon that inherited the lock: killing it releases the lock
      echo "Killing dbsrt-server daemons that may hold the spawn lock..."
      pkill -KILL -u "$(id -u)" -f -- "${PATTERN}" || true
      sleep 1
      (
        lock_or_fail
        touch "${STOP_MARKER}"
      ) 9>"${SPAWN_LOCK}"
    fi
    # Re-signal every round: also catches a spawn that exec'd just before the marker was set
    for _ in $(seq 1 "${SHUTDOWN_TIMEOUT_SEC}"); do
      [[ -n "$(daemon_pids)" ]] || break
      pkill -TERM -u "$(id -u)" -f -- "${PATTERN}" || true
      sleep 1
    done
    if [[ -n "$(daemon_pids)" ]]; then
      echo "Daemon(s) ignored SIGTERM for ${SHUTDOWN_TIMEOUT_SEC}s, sending SIGKILL..."
      pkill -KILL -u "$(id -u)" -f -- "${PATTERN}" || true
      sleep 1
    fi
    rm -f "${LEGACY_PID_FILE}"
    if [[ -n "$(daemon_pids)" ]]; then
      echo "❌ dbsrt-server still running (PID: $(daemon_pids | paste -sd ' '))."
      exit 1
    fi
    echo "✓ dbsrt-server stopped; gateway and cron respawn disabled until start."
    ;;

  status)
    PIDS="$(daemon_pids)"
    COUNT=$(grep -c . <<<"${PIDS}" || true)
    if [[ "${COUNT}" -eq 0 && -e "${STOP_MARKER}" ]]; then
      echo "○ dbsrt-server is stopped by operator (respawn disabled until start)."
      exit 3
    fi
    if [[ "${COUNT}" -eq 0 ]]; then
      echo "○ dbsrt-server is not running (next request or cron watchdog respawns it)."
      exit 3
    fi
    if [[ -e "${STOP_MARKER}" ]]; then
      echo "⚠️ stop marker present but a daemon runs: run stop again."
    fi
    echo "● dbsrt-server is running (${COUNT} process(es))."
    ps -o pid,etimes,%cpu,rss,comm -p "$(paste -sd , <<<"${PIDS}")" || true
    if [[ "${COUNT}" -gt 1 ]]; then
      echo "❌ ${COUNT} daemons on ${SOCKET}: expected exactly 1."
      exit 1
    fi
    ;;

  pids)
    daemon_pids
    ;;

  watchdog)
    mkdir -p "${BASE}/run"
    (
      lock_or_fail
      [[ -e "${STOP_MARKER}" || -n "$(daemon_pids)" ]] || spawn
    ) 9>"${SPAWN_LOCK}"
    ;;

  *)
    echo "usage: $0 <start|stop|status|pids|watchdog> [base_dir] [socket]" >&2
    exit 2
    ;;
esac
