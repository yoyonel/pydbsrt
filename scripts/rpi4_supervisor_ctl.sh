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
LOG_FILE="${REMOTE_DIR}/logs/rpi4_supervisor.log"
# Lifecycle is owned by the systemd user unit (Restart=always): never spawn the script directly
SERVICE_NAME="${SERVICE_NAME:-dbsrt-supervisor.service}"

run_ssh() {
  ssh -o BatchMode=yes -o ConnectTimeout="${DEFAULT_CONNECT_TIMEOUT}" "${RPI_HOST}" "$@"
}

run_systemctl() {
  run_ssh "systemctl --user $*"
}

case "${ACTION}" in
  status)
    run_systemctl "status --no-pager --lines=0 ${SERVICE_NAME}" || true
    echo "--- Last activity logs ---"
    run_ssh "tail -n 5 ${LOG_FILE} 2>/dev/null || true"
    ;;

  start | restart)
    echo "=== ${ACTION^} supervisor (${SERVICE_NAME}) on ${RPI_HOST} ==="
    run_systemctl "daemon-reload"
    run_systemctl "${ACTION} ${SERVICE_NAME}"
    sleep 1
    if run_systemctl "is-active --quiet ${SERVICE_NAME}"; then
      echo "✓ RPi4 Supervisor Agent active on ${RPI_HOST} (PID $(run_systemctl "show -p MainPID --value ${SERVICE_NAME}"))."
    else
      echo "⚠️ ${SERVICE_NAME} not active. Inspecting logs:"
      run_ssh "tail -n ${DEFAULT_TAIL_LINES} ${LOG_FILE} 2>/dev/null || true"
      exit 1
    fi
    ;;

  stop)
    echo "=== Stopping supervisor (${SERVICE_NAME}) on ${RPI_HOST} ==="
    run_systemctl "stop ${SERVICE_NAME}"
    echo "✓ RPi4 Supervisor stopped on ${RPI_HOST}."
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
