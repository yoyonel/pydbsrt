#!/usr/bin/env bash
# Manages the Freebox CIFS mounts and the HTTP Range gateway on the RPi4.
# o2switch reaches the gateway through a Freebox NAT rule restricted to its source IP:
# SSH reverse tunnels are killed by o2switch after ~20s, so none is used.
set -euo pipefail

# Named Operational Constants
DEFAULT_RPI_HOST="rpi4"
DEFAULT_RPI_HOSTNAME="raspberrypi"
DEFAULT_REMOTE_BASE="dbsrt"
GATEWAY_SERVICE="dbsrt-freebox-gateway.service"
# Mount points must be declared in the RPi4 /etc/fstab (read-only, credentials file), see usage()
FREEBOX_MOUNTS=(/mnt/freebox/Disque_1 /mnt/freebox/Disque_2 /mnt/freebox/Disque_1_1)

ACTION="${1:-status}"
RPI_HOST="${RPI_HOST:-${DEFAULT_RPI_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"

is_local_rpi() {
  [[ "$(hostname 2>/dev/null)" == "${DEFAULT_RPI_HOSTNAME}" ]]
}

run_rpi() {
  if is_local_rpi; then
    bash -c "$*"
  else
    ssh -o BatchMode=yes -o ConnectTimeout=10 "${RPI_HOST}" "$@"
  fi
}

usage() {
  cat <<'EOF'
Usage: rpi4_freebox_ctl.sh {install|start|stop|restart|status|mount|unmount}

One-time RPi4 setup (as root), credentials never pass on a command line:
  printf 'username=freebox\npassword=<secret>\n' > /etc/freebox.cred && chmod 600 /etc/freebox.cred
  /etc/fstab, one line per share (\040 encodes spaces):
  //192.168.1.254/Disque\0401 /mnt/freebox/Disque_1 cifs ro,credentials=/etc/freebox.cred,uid=1000,gid=1000,iocharset=utf8,vers=1.0,_netdev,nofail 0 0
  (same for "Disque 2" -> Disque_2 and "Disque 1 1" -> Disque_1_1)
EOF
}

mount_shares() {
  echo "=== Mounting Freebox volumes on ${RPI_HOST} (fstab entries) ==="
  local mp
  for mp in "${FREEBOX_MOUNTS[@]}"; do
    if run_rpi "mountpoint -q '${mp}'"; then
      echo "✓ Already mounted: ${mp}"
    else
      run_rpi "sudo mkdir -p '${mp}' && sudo mount '${mp}'" || {
        echo "❌ Cannot mount ${mp}: add it to /etc/fstab on ${RPI_HOST}."
        usage
        exit 1
      }
      echo "✓ Mounted: ${mp}"
    fi
  done
}

unmount_shares() {
  echo "=== Unmounting Freebox volumes on ${RPI_HOST} ==="
  local mp
  for mp in "${FREEBOX_MOUNTS[@]}"; do
    run_rpi "! mountpoint -q '${mp}' || sudo umount '${mp}'"
  done
  echo "✓ Freebox volumes unmounted."
}

install_gateway() {
  echo "=== Installing ${GATEWAY_SERVICE} on ${RPI_HOST} ==="
  if is_local_rpi; then
    echo "❌ Run install from the workstation (it copies the repo files to ${RPI_HOST})."
    exit 1
  fi
  run_rpi "mkdir -p ${REMOTE_BASE}/scripts ${REMOTE_BASE}/logs .config/systemd/user"
  rsync -az scripts/rpi4_freebox_gateway.py "${RPI_HOST}:${REMOTE_BASE}/scripts/"
  rsync -az "deploy/rpi4/${GATEWAY_SERVICE}" "${RPI_HOST}:.config/systemd/user/${GATEWAY_SERVICE}"
  run_rpi "systemctl --user daemon-reload && systemctl --user enable ${GATEWAY_SERVICE}"
  echo "✓ Installed and enabled ${GATEWAY_SERVICE} (run 'restart' to load new code)."
}

status_services() {
  echo "=== Freebox mounts & gateway on ${RPI_HOST} ==="
  run_rpi "mount -t cifs | grep -o 'on /mnt/freebox/[^ ]*' || echo '○ No Freebox mount'"
  run_rpi "systemctl --user is-active ${GATEWAY_SERVICE}" || true
}

case "${ACTION}" in
  install) install_gateway ;;
  start)
    mount_shares
    run_rpi "systemctl --user start ${GATEWAY_SERVICE}"
    ;;
  stop) run_rpi "systemctl --user stop ${GATEWAY_SERVICE}" ;;
  restart)
    mount_shares
    run_rpi "systemctl --user restart ${GATEWAY_SERVICE}"
    ;;
  status) status_services ;;
  mount) mount_shares ;;
  unmount) unmount_shares ;;
  *)
    usage
    exit 1
    ;;
esac
