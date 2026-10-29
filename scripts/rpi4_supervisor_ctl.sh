#!/usr/bin/env bash
# Controls deployment, lifecycle, and telemetry of the autonomous RPi4 supervisor watchdog
set -euo pipefail

# Named operational constants
DEFAULT_RPI_HOST="rpi4"
DEFAULT_REMOTE_BASE="dbsrt"
DEFAULT_CONNECT_TIMEOUT=5
DEFAULT_TAIL_LINES=20

ACTION="${1:-status}"
RPI_HOST="${RPI_HOST:-${DEFAULT_RPI_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"

REMOTE_DIR="\$HOME/${REMOTE_BASE}"
PID_FILE="${REMOTE_DIR}/run/rpi4_supervisor.pid"
LOG_FILE="${REMOTE_DIR}/logs/rpi4_supervisor.log"

run_ssh() {
  ssh -o BatchMode=yes -o ConnectTimeout="${DEFAULT_CONNECT_TIMEOUT}" "${RPI_HOST}" "$@"
}

case "${ACTION}" in
  status)
    # Check if PID file exists and process is alive
    if run_ssh "test -f ${PID_FILE} && kill -0 \$(cat ${PID_FILE}) 2>/dev/null"; then
      PID=$(run_ssh "cat ${PID_FILE}")
      echo "● RPi4 Supervisor Agent is RUNNING on ${RPI_HOST} (PID ${PID})"
      echo "--- Last activity logs ---"
      run_ssh "tail -n 5 ${LOG_FILE} 2>/dev/null || true"
    else
      # Fallback check if python supervisor script is running without PID file
      REMOTE_PID=$(run_ssh "pgrep -f 'rpi4_o2switch_supervisor.py' 2>/dev/null | head -n 1" || true)
      if [[ -n "${REMOTE_PID}" ]]; then
        echo "● RPi4 Supervisor Agent is RUNNING on ${RPI_HOST} (PID ${REMOTE_PID}, PID file missing)"
        echo "--- Last activity logs ---"
        run_ssh "tail -n 5 ${LOG_FILE} 2>/dev/null || true"
      else
        echo "○ RPi4 Supervisor Agent is STOPPED on ${RPI_HOST}"
      fi
    fi
    ;;

  start)
    echo "=== Starting autonomous supervisor on ${RPI_HOST} ==="
    if run_ssh "test -f ${PID_FILE} && kill -0 \$(cat ${PID_FILE}) 2>/dev/null"; then
      PID=$(run_ssh "cat ${PID_FILE}")
      echo "⚠️ RPi4 Supervisor is already running on ${RPI_HOST} (PID ${PID})."
      exit 0
    fi

    run_ssh "cd ${REMOTE_DIR} && mkdir -p run logs && nohup /usr/bin/python3 -u scripts/rpi4_o2switch_supervisor.py </dev/null >> ${LOG_FILE} 2>&1 &"
    sleep 1

    if run_ssh "test -f ${PID_FILE} && kill -0 \$(cat ${PID_FILE}) 2>/dev/null"; then
      PID=$(run_ssh "cat ${PID_FILE}")
      echo "✓ RPi4 Supervisor Agent successfully started on ${RPI_HOST} (PID ${PID})."
    else
      echo "⚠️ Failed verifying supervisor startup. Inspecting logs:"
      run_ssh "tail -n ${DEFAULT_TAIL_LINES} ${LOG_FILE} 2>/dev/null || true"
      exit 1
    fi
    ;;

  stop)
    echo "=== Stopping autonomous supervisor on ${RPI_HOST} ==="
    run_ssh "
      if [[ -f ${PID_FILE} ]]; then
        kill \$(cat ${PID_FILE}) 2>/dev/null || true
        rm -f ${PID_FILE}
      fi
      pkill -f 'rpi4_o2switch_supervisor.py' 2>/dev/null || true
    "
    echo "✓ RPi4 Supervisor stopped on ${RPI_HOST}."
    ;;

  restart)
    "$0" stop
    sleep 1
    "$0" start
    ;;

  logs)
    echo "=== Tailing live supervisor logs on ${RPI_HOST} (${LOG_FILE}) ==="
    ssh -t -o BatchMode=yes "${RPI_HOST}" "tail -f ${LOG_FILE}"
    ;;

  *)
    echo "Usage: $0 {status|start|stop|restart|logs}"
    exit 1
    ;;
esac
