#!/usr/bin/env bash
# Controls deployment, lifecycle, and telemetry of the autonomous RPi4 supervisor watchdog
set -euo pipefail

# Named operational constants
DEFAULT_RPI_HOST="rpi4"
DEFAULT_REMOTE_BASE="dbsrt"
DEFAULT_CONNECT_TIMEOUT=5
DEFAULT_TAIL_LINES=20

ACTION="${1:-status}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel 2>/dev/null)"; then
  if [[ -f "${SCRIPT_DIR}/../../../Taskfile.yml" ]]; then
    REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
  elif [[ -d "${SCRIPT_DIR}/../data" || -f "${SCRIPT_DIR}/../bin/python3" || -f "${SCRIPT_DIR}/o2switch_batch_worker.py" ]]; then
    REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
  else
    REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
  fi
fi

# Load optional unversioned local environment file
if [[ -f "${SCRIPT_DIR}/../local/load_env.sh" ]]; then
  # shellcheck source=/dev/null
  source "${SCRIPT_DIR}/../local/load_env.sh"
elif [[ -f "${SCRIPT_DIR}/../o2switch/load_env.sh" ]]; then
  # shellcheck source=/dev/null
  source "${SCRIPT_DIR}/../o2switch/load_env.sh"
elif [[ -f "${REPO_ROOT}/scripts/infra/local/load_env.sh" ]]; then
  # shellcheck source=/dev/null
  source "${REPO_ROOT}/scripts/infra/local/load_env.sh"
fi

RPI_HOST="${RPI_HOST:-${DEFAULT_RPI_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"

REMOTE_DIR="\$HOME/${REMOTE_BASE}"
LOG_FILE="${REMOTE_DIR}/logs/rpi4_supervisor.log"
# Lifecycle is owned by the systemd user unit (Restart=always): never spawn the script directly
SERVICE_NAME="${SERVICE_NAME:-dbsrt-supervisor.service}"
UNIT_FILE="${REPO_ROOT}/deploy/rpi4/dbsrt-supervisor.service"
# Scripts the supervisor runs on the RPi4 (it drives o2switch through o2switch_ctl.sh)
SUPERVISOR_SCRIPTS=(
  "${REPO_ROOT}/scripts/infra/rpi4/rpi4_o2switch_supervisor.py"
  "${REPO_ROOT}/scripts/infra/o2switch/o2switch_ctl.sh"
  "${REPO_ROOT}/scripts/infra/local/load_env.sh"
  "${REPO_ROOT}/scripts/workers/o2switch_batch_worker.py"
  "${REPO_ROOT}/scripts/ingestion/phash_dedupe.py"
  "${REPO_ROOT}/scripts/ingestion/phash_names.py"
  "${REPO_ROOT}/scripts/infra/rpi4/freebox_batch_ctl.sh"
  "${REPO_ROOT}/scripts/ingestion/scan_freebox_targets.py"
  "${REPO_ROOT}/scripts/infra/rpi4/rpi4_supervisor_ctl.sh"
)

run_ssh() {
  ssh -o BatchMode=yes -o ConnectTimeout="${DEFAULT_CONNECT_TIMEOUT}" "${RPI_HOST}" "$@"
}

run_systemctl() {
  run_ssh "systemctl --user $*"
}

sync_scripts() {
  rsync -az -e "ssh -o BatchMode=yes -o ConnectTimeout=${DEFAULT_CONNECT_TIMEOUT}" \
    "${SUPERVISOR_SCRIPTS[@]}" "${RPI_HOST}:${REMOTE_BASE}/scripts/"
}

case "${ACTION}" in
  install)
    echo "=== Installing supervisor scripts and ${SERVICE_NAME} on ${RPI_HOST} ==="
    run_ssh "mkdir -p ${REMOTE_DIR}/scripts ${REMOTE_DIR}/logs ${REMOTE_DIR}/run \$HOME/.config/systemd/user"
    sync_scripts
    rsync -az -e "ssh -o BatchMode=yes -o ConnectTimeout=${DEFAULT_CONNECT_TIMEOUT}" \
      "${UNIT_FILE}" "${RPI_HOST}:.config/systemd/user/${SERVICE_NAME}"
    run_systemctl "daemon-reload"
    run_systemctl "enable ${SERVICE_NAME}"
    echo "✓ Installed and enabled ${SERVICE_NAME} (run 'restart' to load new code)."
    ;;

  sync)
    # Scripts only: the RPi4 pushes its own o2switch_batch_worker.py on every batch launch
    echo "=== Syncing supervisor scripts to ${RPI_HOST} ==="
    sync_scripts
    echo "local  $(sha256sum "${REPO_ROOT}/scripts/workers/o2switch_batch_worker.py" | cut -c1-16)"
    echo "remote $(run_ssh "sha256sum ${REMOTE_BASE}/scripts/o2switch_batch_worker.py" | cut -c1-16)"
    ;;

  status)
    run_systemctl "status --no-pager --lines=0 ${SERVICE_NAME}" || true
    echo "--- Last activity logs ---"
    run_ssh "tail -n 5 ${LOG_FILE} 2>/dev/null || true"
    ;;

  start | restart)
    echo "=== ${ACTION^} supervisor (${SERVICE_NAME}) on ${RPI_HOST} ==="
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
    ssh -t -o BatchMode=yes -o ConnectTimeout="${DEFAULT_CONNECT_TIMEOUT}" "${RPI_HOST}" "tail -f ${LOG_FILE}"
    ;;

  *)
    echo "Usage: $0 {install|sync|status|start|stop|restart|logs}"
    exit 1
    ;;
esac
