#!/usr/bin/env bash
# Controls deployment, sync, lifecycle, and telemetry of dbsrt-server on o2switch (Cloud cPanel)
set -euo pipefail

# Named operational constants
DEFAULT_O2SWITCH_HOST="o2switch"
DEFAULT_REMOTE_BASE="dbsrt"
DEFAULT_REMOTE_PUBLIC_HTML="public_html/dbsrt"
DEFAULT_SERVER_SOCKET_REL="run/dbsrt-server.sock"
DEFAULT_LOCAL_TEST_PORT=8769
DEFAULT_HEALTHCHECK_MAX_RETRIES=20
DEFAULT_HEALTHCHECK_SLEEP_SEC=0.5
DEFAULT_SHUTDOWN_TIMEOUT_SEC=10
DEFAULT_DEMO_SAMPLE_LINES=8
DEFAULT_CURL_TIMEOUT_SEC=5
# CloudLinux LVE I/O limit is 42 MiB/s on o2switch.
# Cap rsync bandwidth to 20,000 KB/s (~20 MB/s) to guarantee zero I/O faults.
DEFAULT_RSYNC_BWLIMIT_KBPS=20000
TARGET_TRIPLE="x86_64-unknown-linux-musl"
TARGET_DIR="target/${TARGET_TRIPLE}/release"
SERVER_BIN="${TARGET_DIR}/dbsrt-server"
CLI_BIN="${TARGET_DIR}/dbsrt"

ACTION="${1:-status}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Load optional unversioned local environment file
# shellcheck source=scripts/load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_local_env "${REPO_ROOT}"

DEFAULT_NAS_PASS_ENTRY="services/synology/nas_pass"
DEFAULT_API_KEY_PASS_ENTRY="services/pydbsrt/o2switch_api_key"
DEFAULT_READONLY_KEY_PASS_ENTRY="services/pydbsrt/o2switch_readonly_key"
# Worker concurrency bounds, kept in sync with MIN/MAX_CONCURRENT_WORKERS in o2switch_batch_worker.py
MIN_BATCH_WORKERS=1
MAX_BATCH_WORKERS=8

# NAS endpoint comes from .env / environment only (no versioned default)
NAS_HOST="${NAS_HOST:-}"
NAS_USER="${NAS_USER:-}"
NAS_PASS_ENTRY="${NAS_PASS_ENTRY:-${DEFAULT_NAS_PASS_ENTRY}}"
API_KEY_PASS_ENTRY="${API_KEY_PASS_ENTRY:-${DEFAULT_API_KEY_PASS_ENTRY}}"
READONLY_KEY_PASS_ENTRY="${READONLY_KEY_PASS_ENTRY:-${DEFAULT_READONLY_KEY_PASS_ENTRY}}"

O2SWITCH_HOST="${O2SWITCH_HOST:-${DEFAULT_O2SWITCH_HOST}}"
REMOTE_BASE="${REMOTE_BASE:-${DEFAULT_REMOTE_BASE}}"
# o2switch kills TCP listeners within seconds, not Unix sockets (path relative to the remote $HOME)
SERVER_SOCKET="${SERVER_SOCKET:-${REMOTE_BASE}/${DEFAULT_SERVER_SOCKET_REL}}"
REMOTE_PUBLIC_HTML="${REMOTE_PUBLIC_HTML:-${DEFAULT_REMOTE_PUBLIC_HTML}}"
O2SWITCH_URL="${O2SWITCH_URL:-}"
O2SWITCH_API_KEY="${O2SWITCH_API_KEY:-}"
O2SWITCH_READONLY_KEY="${O2SWITCH_READONLY_KEY:-}"
# Remote snippet emitting the loopback auth header from the master key file; piped into
# `curl -H @-` so the key stays out of every argv (the server enforces --api-key-file)
REMOTE_AUTH_HEADER="printf 'Authorization: Bearer %s\\n' \"\$(grep -m 1 -v -e '^#' -e '^[[:space:]]*\$' ${REMOTE_BASE}/etc/api_key.secret)\""

check_ssh() {
  if ! ssh -o BatchMode=yes -o ConnectTimeout="${DEFAULT_CURL_TIMEOUT_SEC}" "${O2SWITCH_HOST}" "true" 2>/dev/null; then
    echo "❌ Cannot connect to '${O2SWITCH_HOST}' via SSH."
    echo "   Ensure ~/.ssh/config contains a Host block for '${O2SWITCH_HOST}', or set O2SWITCH_HOST=user@yourdomain.com."
    exit 1
  fi
}

# Reads a secret from the pass store only when an action needs it
pass_secret() {
  if command -v pass >/dev/null 2>&1; then
    pass show "$1" 2>/dev/null || true
  fi
}

# Generates a 256-bit key, stores it in pass first, then installs it remotely (mode 0600).
# Sets NEW_KEY for the caller's live verification.
install_new_key() {
  local pass_entry="$1" remote_file="$2"
  check_ssh
  if ! command -v pass >/dev/null 2>&1; then
    echo "❌ 'pass' CLI is required to store the new key (${pass_entry}). Aborting before any change."
    exit 1
  fi
  NEW_KEY=$(openssl rand -hex 32)
  KEY_FINGERPRINT=$(printf "%s" "${NEW_KEY}" | sha256sum | cut -d' ' -f1 | head -c 16)
  echo "-> Generated new 256-bit key (fingerprint: ${KEY_FINGERPRINT})."

  # Store locally first: a key installed remotely but lost locally would lock us out
  if ! printf "%s\n" "${NEW_KEY}" | pass insert -m -f "${pass_entry}" >/dev/null; then
    echo "❌ Failed to store the new key in pass (${pass_entry}). Remote key left unchanged."
    exit 1
  fi
  echo "✓ Stored new key in local password store (${pass_entry})."

  # Key travels over stdin, never in the remote command line (visible in ps)
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "umask 077 && mkdir -p $(printf '%q' "${REMOTE_BASE}/etc") && cat > $(printf '%q' "${REMOTE_BASE}/${remote_file}")" <<<"${NEW_KEY}"
  echo "✓ Installed new key on remote host (${REMOTE_BASE}/${remote_file}, mode 0600)."
}

# Prints the HTTP code of a public request authenticated with the key given on stdin
public_http_code() {
  local method="$1" url="$2"
  curl -s -o /dev/null -w "%{http_code}" -X "${method}" -H @- "${url}" 2>/dev/null || true
}

# Deploys the remote lifecycle helper (also run by the cron watchdog), then runs one of its actions
remote_server_ctl() {
  local action="$1"
  rsync -a --chmod=F755 "${REPO_ROOT}/deploy/o2switch/dbsrt_server_ctl.sh" "${O2SWITCH_HOST}:${REMOTE_BASE}/bin/"
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "bash $(printf '%q ' "${REMOTE_BASE}/bin/dbsrt_server_ctl.sh" "${action}" "${REMOTE_BASE}" "${SERVER_SOCKET}")"
}

# Stops the remote process recorded in a PID file (SIGTERM, then SIGKILL after a timeout)
remote_stop_pidfile() {
  local pid_file="$1"
  # shellcheck disable=SC2029
  ssh "${O2SWITCH_HOST}" "bash -s -- $(printf '%q ' "${REMOTE_BASE}/${pid_file}" "${DEFAULT_SHUTDOWN_TIMEOUT_SEC}")" <<'REMOTE_SCRIPT'
PID_FILE="$1"
TIMEOUT="$2"
if [[ ! -f "${PID_FILE}" ]]; then
  echo "No PID file found at ${PID_FILE}."
  exit 0
fi
PID=$(cat "${PID_FILE}")
if kill -0 "${PID}" 2>/dev/null; then
  echo "Sending SIGTERM to PID ${PID}..."
  kill -TERM "${PID}"
  for _ in $(seq 1 "${TIMEOUT}"); do
    if ! kill -0 "${PID}" 2>/dev/null; then
      break
    fi
    sleep 1
  done
  if kill -0 "${PID}" 2>/dev/null; then
    echo "Process did not terminate gracefully, sending SIGKILL..."
    kill -KILL "${PID}" || true
  fi
  echo "✓ Stopped process ${PID}."
fi
rm -f "${PID_FILE}"
REMOTE_SCRIPT
}

case "${ACTION}" in
  build)
    echo "=== Cross-compiling static standalone binaries (${TARGET_TRIPLE}) ==="
    cargo build --release --target "${TARGET_TRIPLE}" --bin dbsrt --bin dbsrt-server
    echo "=== Stripping binaries to minimize transfer and disk footprint ==="
    strip "${CLI_BIN}" "${SERVER_BIN}"
    echo "✓ Built and stripped static binaries:"
    ls -lh "${CLI_BIN}" "${SERVER_BIN}"
    file "${CLI_BIN}" "${SERVER_BIN}"
    ;;

  test-local)
    if [[ ! -f "${SERVER_BIN}" ]]; then
      echo "Binary not found at ${SERVER_BIN}. Building first..."
      "$0" build
    fi
    echo "=== Testing static musl dbsrt-server in Zero-DB mode locally (port ${DEFAULT_LOCAL_TEST_PORT}) ==="
    LOCAL_PID_FILE=$(mktemp)
    LOCAL_LOG_FILE=$(mktemp)

    "${SERVER_BIN}" \
      --host 127.0.0.1 \
      --port "${DEFAULT_LOCAL_TEST_PORT}" \
      --dbphash data/catalog.dbphash \
      --subtitles-dir data/SRT \
      --phash-dir data/__PHASH__ >"${LOCAL_LOG_FILE}" 2>&1 &
    TEST_PID=$!
    echo "${TEST_PID}" >"${LOCAL_PID_FILE}"

    cleanup_local() {
      if kill -0 "${TEST_PID}" 2>/dev/null; then
        kill "${TEST_PID}" 2>/dev/null || true
        wait "${TEST_PID}" 2>/dev/null || true
      fi
      rm -f "${LOCAL_PID_FILE}" "${LOCAL_LOG_FILE}"
    }
    trap cleanup_local EXIT

    echo "Waiting for healthcheck on http://127.0.0.1:${DEFAULT_LOCAL_TEST_PORT}/v1/health..."
    HEALTHY=false
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      if curl -s -f "http://127.0.0.1:${DEFAULT_LOCAL_TEST_PORT}/v1/health" >/dev/null 2>&1; then
        HEALTHY=true
        break
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done

    if [[ "${HEALTHY}" != "true" ]]; then
      echo "❌ Local healthcheck failed. Server output:"
      cat "${LOCAL_LOG_FILE}"
      exit 1
    fi
    echo "✓ Local healthcheck OK."

    echo "=== Querying /v1/status ==="
    STATUS_JSON=$(curl -s -f "http://127.0.0.1:${DEFAULT_LOCAL_TEST_PORT}/v1/status")
    echo "${STATUS_JSON}" | jq '{status: .status, catalog_loaded: .catalog_loaded, total_medias: .total_medias, base_cuts_count: .base_cuts_count}'

    echo "=== Querying /v1/recognize with sample fingerprint ==="
    RESP=$(curl -s -f -X POST "http://127.0.0.1:${DEFAULT_LOCAL_TEST_PORT}/v1/recognize" \
      -H "Content-Type: application/json" \
      -d '{
      "query_fingerprint": [
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b",
        "0xe9fd17223e1c0c4b"
      ],
      "fps": 25.0,
      "language": "fr"
    }')
    echo "${RESP}" | jq .
    CANDIDATE_ID=$(echo "${RESP}" | jq -r '.candidates[0].candidate_id // empty')
    if [[ "${CANDIDATE_ID}" =~ ^sub:sha256:(.+) ]]; then
      HASH="${BASH_REMATCH[1]}"
      echo "=== Testing CAS Subtitle Retrieval (/v1/subtitles/${HASH}) ==="
      SRT_OUT=$(curl -s -f "http://127.0.0.1:${DEFAULT_LOCAL_TEST_PORT}/v1/subtitles/${HASH}")
      (echo "${SRT_OUT}" | head -n "${DEFAULT_DEMO_SAMPLE_LINES}") || true
      echo "✓ CAS Subtitle retrieval verified!"
    fi

    echo "=== Checking Memory Footprint (RSS) ==="
    ps -o pid,rss,vsz,comm -p "${TEST_PID}"
    echo "✓ Local static musl Zero-DB execution empirically verified!"
    ;;

  sync)
    check_ssh
    if [[ ! -f "${SERVER_BIN}" || ! -f "${CLI_BIN}" ]]; then
      echo "Binaries not found. Building first..."
      "$0" build
    fi
    echo "=== Synchronizing standalone stack to ${O2SWITCH_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "mkdir -p ${REMOTE_BASE}/bin ${REMOTE_BASE}/data ${REMOTE_BASE}/run ${REMOTE_BASE}/logs ${REMOTE_PUBLIC_HTML} ${REMOTE_BASE}/scripts"

    echo "-> Transferring static binaries..."
    rsync -avz --progress --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${CLI_BIN}" "${SERVER_BIN}" "${O2SWITCH_HOST}:${REMOTE_BASE}/bin/"
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "chmod +x ${REMOTE_BASE}/bin/dbsrt ${REMOTE_BASE}/bin/dbsrt-server"
    rsync -a --chmod=F755 "deploy/o2switch/dbsrt_server_ctl.sh" "${O2SWITCH_HOST}:${REMOTE_BASE}/bin/"

    echo "-> Transferring batch worker scripts..."
    if [[ -f "scripts/o2switch_batch_worker.py" ]]; then
      rsync -avz --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "scripts/o2switch_batch_worker.py" "${O2SWITCH_HOST}:${REMOTE_BASE}/scripts/"
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "chmod +x ${REMOTE_BASE}/scripts/o2switch_batch_worker.py"
    fi

    echo "-> Transferring Zero-DB catalog and SRT store (bwlimit ${DEFAULT_RSYNC_BWLIMIT_KBPS} KB/s)..."
    # The remote catalog is packed on o2switch at start: the local one only seeds a fresh host
    if [[ -f "data/catalog.dbphash" ]]; then
      rsync -avz --progress --ignore-existing --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "data/catalog.dbphash" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/"
    fi
    if [[ -d "data/SRT" ]]; then
      rsync -avz --progress --update --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "data/SRT/" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/SRT/"
    fi

    echo "-> Deploying reverse-proxy gateway (index.php + .htaccess)..."
    if [[ -f "deploy/o2switch/index.php" ]]; then
      rsync -avz --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "deploy/o2switch/index.php" "${O2SWITCH_HOST}:${REMOTE_PUBLIC_HTML}/index.php"
    fi
    if [[ -f "deploy/o2switch/.htaccess" ]]; then
      rsync -avz --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "deploy/o2switch/.htaccess" "${O2SWITCH_HOST}:${REMOTE_PUBLIC_HTML}/.htaccess"
    fi

    echo "✓ Stack synchronized to ${O2SWITCH_HOST}."
    ;;

  start)
    check_ssh
    echo "=== Starting dbsrt-server daemon on ${O2SWITCH_HOST} ==="
    remote_server_ctl start

    echo "Waiting for healthcheck on unix:${SERVER_SOCKET} via SSH..."
    HEALTHY=false
    for _ in $(seq 1 "${DEFAULT_HEALTHCHECK_MAX_RETRIES}"); do
      # shellcheck disable=SC2029
      if ssh "${O2SWITCH_HOST}" "curl -s -f --unix-socket ${SERVER_SOCKET} http://localhost/v1/health" >/dev/null 2>&1; then
        HEALTHY=true
        break
      fi
      sleep "${DEFAULT_HEALTHCHECK_SLEEP_SEC}"
    done

    if [[ "${HEALTHY}" != "true" ]]; then
      echo "❌ Remote healthcheck failed. Tail of remote log:"
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "tail -n 20 ${REMOTE_BASE}/logs/dbsrt-server.log"
      exit 1
    fi
    echo "✓ Remote dbsrt-server is healthy on ${O2SWITCH_HOST}."
    ;;

  stop)
    check_ssh
    echo "=== Stopping dbsrt-server daemon on ${O2SWITCH_HOST} ==="
    # A watchdog line from before dbsrt_server_ctl.sh ignores the stop marker and would respawn
    # shellcheck disable=SC2029
    if ssh "${O2SWITCH_HOST}" "crontab -l 2>/dev/null | grep -F '# dbsrt-server watchdog' | grep -qvF dbsrt_server_ctl.sh"; then
      echo "❌ Legacy cron watchdog would respawn the daemon: run 'task o2switch:setup-cron' first."
      exit 1
    fi
    remote_server_ctl stop
    ;;

  restart)
    "$0" stop
    "$0" start
    ;;

  status)
    check_ssh
    echo "=== Remote Process Status (${O2SWITCH_HOST}) ==="
    STATUS_RC=0
    remote_server_ctl status || STATUS_RC=$?
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "bash -s -- '${REMOTE_BASE}'" <<'REMOTE_SCRIPT'
BASE="$1"
# Same lookup as dbsrt: an ffmpeg next to the binary wins over PATH.
FFMPEG="${BASE}/bin/ffmpeg"
[[ -x "${FFMPEG}" ]] || FFMPEG="$(command -v ffmpeg || true)"
echo "ffmpeg: ${FFMPEG:-not found} ($("${FFMPEG:-false}" -version 2>/dev/null | head -1 || true))"
REMOTE_SCRIPT
    echo ""
    echo "=== Remote API Status (internal unix:${SERVER_SOCKET}) ==="
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "${REMOTE_AUTH_HEADER} | curl -s -f -H @- --unix-socket ${SERVER_SOCKET} http://localhost/v1/status || echo 'API unreachable'" | jq . 2>/dev/null || true
    # 0 = one daemon, 3 = not running (stopped or awaiting respawn), 1 = duplicate daemons
    exit "${STATUS_RC}"
    ;;

  logs)
    check_ssh
    # shellcheck disable=SC2029
    ssh -t "${O2SWITCH_HOST}" "tail -f ${REMOTE_BASE}/logs/dbsrt-server.log"
    ;;

  demo)
    echo "=== Running Recognition Demo against o2switch ==="
    if [[ -n "${O2SWITCH_URL}" ]]; then
      O2SWITCH_API_KEY="${O2SWITCH_API_KEY:-$(pass_secret "${API_KEY_PASS_ENTRY}")}"
      TARGET_ENDPOINT="${O2SWITCH_URL}/v1/recognize"
      echo "Querying public HTTPS URL: ${TARGET_ENDPOINT}"
      # Key travels as a header on stdin (curl -H @-), never in argv (visible in ps)
      AUTH_LINE=""
      if [[ -n "${O2SWITCH_API_KEY}" ]]; then
        AUTH_LINE="X-API-Key: ${O2SWITCH_API_KEY}"
      fi
      RESP=$(printf '%s\n' "${AUTH_LINE}" | curl -s -f -X POST "${TARGET_ENDPOINT}" \
        -H "Content-Type: application/json" \
        -H @- \
        -d '{
        "query_fingerprint": [
          "0xe9fd17223e1c0c4b",
          "0xe9fd17223e1c0c4b",
          "0xe9fd17223e1c0c4b",
          "0xe9fd17223e1c0c4b",
          "0xe9fd17223e1c0c4b"
        ],
        "fps": 25.0,
        "language": "fr"
      }')
      echo "${RESP}" | jq .
      CANDIDATE_ID=$(echo "${RESP}" | jq -r '.candidates[0].candidate_id // empty')
      if [[ "${CANDIDATE_ID}" =~ ^sub:sha256:(.+) ]]; then
        HASH="${BASH_REMATCH[1]}"
        echo "Fetching subtitle from: ${O2SWITCH_URL}/v1/subtitles/${HASH}"
        (printf '%s\n' "${AUTH_LINE}" | curl -s -f -H @- "${O2SWITCH_URL}/v1/subtitles/${HASH}" | head -n "${DEFAULT_DEMO_SAMPLE_LINES}") || true
      fi
    else
      check_ssh
      echo "Querying internal endpoint via SSH..."
      # shellcheck disable=SC2029
      RESP=$(ssh "${O2SWITCH_HOST}" "${REMOTE_AUTH_HEADER} | curl -s -f -H @- --unix-socket ${SERVER_SOCKET} -X POST http://localhost/v1/recognize \
        -H 'Content-Type: application/json' \
        -d '{\"query_fingerprint\":[\"0xe9fd17223e1c0c4b\",\"0xe9fd17223e1c0c4b\",\"0xe9fd17223e1c0c4b\",\"0xe9fd17223e1c0c4b\",\"0xe9fd17223e1c0c4b\"],\"fps\":25.0,\"language\":\"fr\"}'")
      echo "${RESP}" | jq .
      CANDIDATE_ID=$(echo "${RESP}" | jq -r '.candidates[0].candidate_id // empty')
      if [[ "${CANDIDATE_ID}" =~ ^sub:sha256:(.+) ]]; then
        HASH="${BASH_REMATCH[1]}"
        # shellcheck disable=SC2029
        (ssh "${O2SWITCH_HOST}" "${REMOTE_AUTH_HEADER} | curl -s -f -H @- --unix-socket ${SERVER_SOCKET} http://localhost/v1/subtitles/${HASH}" | head -n "${DEFAULT_DEMO_SAMPLE_LINES}") || true
      fi
    fi
    echo "✓ Demo completed successfully!"
    ;;

  setup-cron)
    check_ssh
    echo "=== Setting up watchdog cron job on ${O2SWITCH_HOST} ==="
    rsync -a --chmod=F755 "${REPO_ROOT}/deploy/o2switch/dbsrt_server_ctl.sh" "${O2SWITCH_HOST}:${REMOTE_BASE}/bin/"
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "bash -s -- '${SERVER_SOCKET}' '${REMOTE_BASE}'" <<'REMOTE_SCRIPT'
SOCKET="$1"
BASE="$2"
CRON_MARKER="# dbsrt-server watchdog"
PACK_MARKER="# dbsrt-server catalog pack"
# Respawns only when no daemon runs and stop has not set the stop marker
CMD="bash \$HOME/${BASE}/bin/dbsrt_server_ctl.sh watchdog ${BASE} ${SOCKET} >> \$HOME/${BASE}/logs/dbsrt-server.log 2>&1"
CRON_LINE="*/5 * * * * ${CMD} ${CRON_MARKER}"
# The on-demand spawn never repacks: fold new batch .phash files into the catalog (atomic mv,
# picked up by the server file watcher and by the next spawn)
D="\$HOME/${BASE}"
PACK_CMD="[ -n \"\$(find ${D}/data/__PHASH__ -name '*.phash' -newer ${D}/data/catalog.dbphash -print -quit)\" ] && nice ${D}/bin/dbsrt pack -p ${D}/data/__PHASH__ -o ${D}/run/catalog.pack.tmp >/dev/null 2>&1 && mv -f ${D}/run/catalog.pack.tmp ${D}/data/catalog.dbphash"
PACK_LINE="*/15 * * * * ${PACK_CMD} ${PACK_MARKER}"

crontab -l 2>/dev/null | grep -v -e "${CRON_MARKER}" -e "${PACK_MARKER}" > /tmp/crontab.tmp || true
printf '%s\n' "${CRON_LINE}" "${PACK_LINE}" >> /tmp/crontab.tmp
crontab /tmp/crontab.tmp
rm -f /tmp/crontab.tmp
echo "✓ Watchdog (every 5 minutes) and catalog pack (every 15 minutes) cron installed:"
crontab -l | grep -e "${CRON_MARKER}" -e "${PACK_MARKER}"
REMOTE_SCRIPT
    ;;

  remove-cron)
    check_ssh
    echo "=== Removing watchdog cron job on ${O2SWITCH_HOST} ==="
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "bash -s" <<'REMOTE_SCRIPT'
crontab -l 2>/dev/null | grep -v -e "# dbsrt-server watchdog" -e "# dbsrt-server catalog pack" > /tmp/crontab.tmp || true
crontab /tmp/crontab.tmp
rm -f /tmp/crontab.tmp
echo "✓ Watchdog and catalog pack cron removed."
REMOTE_SCRIPT
    ;;

  rotate-key)
    echo "=== Rotating dbsrt-server API Secret Key on ${O2SWITCH_HOST} ==="
    install_new_key "${API_KEY_PASS_ENTRY}" "etc/api_key.secret"

    if [[ -z "${O2SWITCH_URL}" ]]; then
      echo "⚠️ O2SWITCH_URL not set: skipping live HTTPS verification."
      exit 0
    fi
    VERIFY_URL="${O2SWITCH_URL}/v1/health"
    echo "-> Verifying live HTTPS healthcheck using new key against ${VERIFY_URL}..."
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -H @- "${VERIFY_URL}" <<<"X-API-Key: ${NEW_KEY}" 2>/dev/null || true)
    if [[ "${HTTP_CODE}" == "200" ]]; then
      echo "✓ New key successfully validated over HTTPS (HTTP 200 OK)."
    else
      echo "⚠️ Healthcheck returned HTTP ${HTTP_CODE} (check domain propagation or logs)."
    fi

    UNAUTH_CODE=$(curl -s -o /dev/null -w "%{http_code}" "${VERIFY_URL}" 2>/dev/null || true)
    if [[ "${UNAUTH_CODE}" == "401" ]]; then
      echo "✓ Rejection of unauthenticated requests strictly confirmed (HTTP 401 Unauthorized)."
    else
      echo "⚠️ Unauthenticated request returned HTTP ${UNAUTH_CODE} (expected 401)."
    fi
    echo "=== API Key rotation completed ==="
    ;;

  rotate-readonly-key)
    echo "=== Rotating read-only (monitoring GET) key on ${O2SWITCH_HOST} ==="
    install_new_key "${READONLY_KEY_PASS_ENTRY}" "etc/api_key_readonly.secret"
    if [[ -z "${O2SWITCH_URL}" ]]; then
      echo "⚠️ O2SWITCH_URL not set: skipping live HTTPS verification."
      exit 0
    fi
    GET_CODE=$(public_http_code GET "${O2SWITCH_URL}/v1/health" <<<"X-API-Key: ${NEW_KEY}")
    POST_CODE=$(public_http_code POST "${O2SWITCH_URL}/v1/catalog/reload" <<<"X-API-Key: ${NEW_KEY}")
    echo "-> Read-only key: GET /v1/health=${GET_CODE} (expect 200), POST /v1/catalog/reload=${POST_CODE} (expect 403)."
    [[ "${GET_CODE}" == "200" && "${POST_CODE}" == "403" ]] || exit 1
    echo "=== Read-only key rotation completed ==="
    ;;

  monitoring)
    # Shared links only need monitoring GETs: hand out the read-only key
    O2SWITCH_API_KEY="${O2SWITCH_READONLY_KEY:-$(pass_secret "${READONLY_KEY_PASS_ENTRY}")}"
    echo "=== dbsrt Web Monitoring Dashboard (o2switch & RPi4) ==="
    if [[ -n "${O2SWITCH_URL}" ]]; then
      echo "  • Dashboard URL : ${O2SWITCH_URL}/monitoring"
      if [[ -n "${O2SWITCH_API_KEY}" ]]; then
        MOBILE_URL="${O2SWITCH_URL}/monitoring?token=${O2SWITCH_API_KEY}"
        echo "  • Mobile Link   : ${MOBILE_URL}"
        echo "  (Ce lien configure automatiquement le cookie sécurisé et nettoie l'URL pour votre smartphone 4G/5G)"
        echo ""
        echo "=== QR Code 4G/5G (Scan Smartphone) ==="
        if command -v qrencode >/dev/null 2>&1; then
          qrencode -t UTF8 "${MOBILE_URL}"
        else
          echo "  (Installez 'qrencode' pour afficher le QR code ASCII directement dans le terminal)"
        fi
      else
        echo "  • Clé secrète   : Non détectée dans .env ou pass store."
      fi
    else
      echo "  • Socket local  : ~/${SERVER_SOCKET} (o2switch)"
      echo "  (Configurez O2SWITCH_URL dans .env pour afficher le lien public)"
    fi
    ;;

  top)
    check_ssh
    shift || true
    echo "=== Connecting to live dbsrt top TUI on ${O2SWITCH_HOST} ==="
    # shellcheck disable=SC2029
    ssh -tt "${O2SWITCH_HOST}" "cd ${REMOTE_BASE} && PATH=\"\$HOME/${REMOTE_BASE}/bin:\$PATH\" RAYON_NUM_THREADS=2 ./bin/dbsrt top ${*:+$(printf '%q ' "$@")}"
    ;;

  pull-catalog)
    check_ssh
    shift || true
    WITH_PHASH=false
    for arg in "$@"; do
      case "${arg}" in
        --with-phash) WITH_PHASH=true ;;
      esac
    done
    echo "=== Pulling processed catalog and artifacts from ${O2SWITCH_HOST} ==="
    mkdir -p data/SRT data/reports
    echo "-> Pulling unified binary catalog (catalog.dbphash)..."
    rsync -avz --progress --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" \
      "${O2SWITCH_HOST}:${REMOTE_BASE}/data/catalog.dbphash" "data/catalog.dbphash.tmp" || true
    if [[ -f "data/catalog.dbphash.tmp" ]]; then
      mv -f "data/catalog.dbphash.tmp" "data/catalog.dbphash"
      echo "✓ catalog.dbphash updated."
    fi
    echo "-> Pulling CAS SRT subtitles..."
    rsync -avz --progress --ignore-existing --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/SRT/" "data/SRT/" || true
    echo "-> Pulling telemetry reports..."
    rsync -avz --progress --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/reports/" "data/reports/" || true
    if [[ "${WITH_PHASH}" == "true" ]]; then
      mkdir -p data/__PHASH__
      echo "-> Pulling loose .phash fingerprints (backup mode)..."
      rsync -avz --progress --ignore-existing --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/__PHASH__/" "data/__PHASH__/" || true
    fi
    echo "✓ Pull completed from ${O2SWITCH_HOST}."
    ;;

  backup-nas)
    check_ssh
    shift || true
    echo "=== Backing up raw .phash fingerprints from ${O2SWITCH_HOST} to NAS/persistent storage ==="
    TARGET_DIR="${1:-}"
    if [[ -z "${TARGET_DIR}" ]]; then
      user_name="${USER:-$(whoami)}"
      candidates=(
        "${NAS_BACKUP_DIR:-}"
        "/media/${user_name}/Synology/phash_backup"
        "/media/${user_name}/Synology/__PHASH__"
        "${HOME}/__SYNOLOGY__/__PHASH__"
        "data/__PHASH__"
      )
      for c in "${candidates[@]}"; do
        if [[ -n "${c}" && -d "${c}" ]]; then
          TARGET_DIR="${c}"
          break
        fi
      done
      TARGET_DIR="${TARGET_DIR:-data/__PHASH__}"
    fi
    mkdir -p "${TARGET_DIR}"
    echo "-> Backing up to ${TARGET_DIR}..."
    rsync -avz --progress --ignore-existing --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/__PHASH__/" "${TARGET_DIR}/" || true
    echo "✓ Backup of raw .phash fingerprints completed to ${TARGET_DIR}."
    ;;

  batch-tvshow | batch-movies)
    check_ssh
    shift || true
    TARGET_TYPE="tvshow"
    LOCAL_USER="${USER:-$(whoami)}"
    LOCAL_SCAN_DIR="${NAS_TVSHOW_DIR:-/media/${LOCAL_USER}/Synology/tvshow}"
    REMOTE_NAS_BASE="/tvshow"
    if [[ "${ACTION}" == "batch-movies" ]]; then
      TARGET_TYPE="movies"
      LOCAL_SCAN_DIR="${NAS_VIDEO_DIR:-/media/${LOCAL_USER}/Synology/video}"
      REMOTE_NAS_BASE="/video"
    fi
    TARGETS_FILE="data/reports/selected_batch_targets.json"
    DAEMON_MODE=false
    WORKER_ARGS=(--nas-base "${REMOTE_NAS_BASE}")
    # Only the selection flags understood by batch_media_add_tvshow.py go to the local scan
    SCAN_ARGS=()
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --daemon | -d) DAEMON_MODE=true ;;
        --fresh-only)
          WORKER_ARGS+=("$1")
          SCAN_ARGS+=("$1")
          ;;
        --limit)
          WORKER_ARGS+=("$1" "${2:?--limit needs a value}")
          SCAN_ARGS+=("$1" "$2")
          shift
          ;;
        *) WORKER_ARGS+=("$1") ;;
      esac
      shift
    done

    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "cd $(printf '%q' "${REMOTE_BASE}") && mkdir -p scripts data/reports data/SRT data/__PHASH__ run logs etc"
    rsync -avz --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" --chmod=F755 "scripts/o2switch_batch_worker.py" "${O2SWITCH_HOST}:${REMOTE_BASE}/scripts/"

    # With a local NAS mount, select targets locally; otherwise the worker scans the NAS over FTP
    if [[ -d "${LOCAL_SCAN_DIR}" ]]; then
      echo "-> Resolving ${TARGET_TYPE} targets locally (${LOCAL_SCAN_DIR})..."
      rm -f "${TARGETS_FILE}"
      if ! uv run python scripts/batch_media_add_tvshow.py --nas-root "${LOCAL_SCAN_DIR}" --save-targets-only \
        --max-episodes-per-series 1000 "${SCAN_ARGS[@]}" >/dev/null; then
        echo "❌ Local target scan failed (see error above)."
        exit 1
      fi
      # The scanner writes nothing when there is nothing to do: send an explicit empty list
      [[ -f "${TARGETS_FILE}" ]] || echo "[]" >"${TARGETS_FILE}"
      echo "-> Transferring selected batch targets..."
      rsync -avz --bwlimit="${DEFAULT_RSYNC_BWLIMIT_KBPS}" "${TARGETS_FILE}" "${O2SWITCH_HOST}:${REMOTE_BASE}/data/reports/"
    else
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "rm -f $(printf '%q' "${REMOTE_BASE}/${TARGETS_FILE}")"
    fi

    # NAS secrets travel over stdin into mode 600 files; hosts without them (RPi4) keep the remote copies
    NAS_PASS="${NAS_PASS:-$(pass_secret "${NAS_PASS_ENTRY}")}"
    if [[ -n "${NAS_PASS}" ]]; then
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "umask 077 && cat > $(printf '%q' "${REMOTE_BASE}/etc/.nas_pass")" <<<"${NAS_PASS}"
    fi
    if [[ -n "${NAS_HOST}" && -n "${NAS_USER}" ]]; then
      # shellcheck disable=SC2029
      printf 'NAS_HOST=%s\nNAS_USER=%s\n' "${NAS_HOST}" "${NAS_USER}" |
        ssh "${O2SWITCH_HOST}" "umask 077 && cat > $(printf '%q' "${REMOTE_BASE}/etc/.nas_config")"
    fi

    REMOTE_ARGS=$(printf '%q ' "${REMOTE_BASE}" "${TARGET_TYPE}" "${WORKER_ARGS[@]}")
    if [[ "${DAEMON_MODE}" == "true" ]]; then
      echo "=== Launching ${TARGET_TYPE} batch ingestion in background daemon on ${O2SWITCH_HOST} ==="
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "bash -s -- ${REMOTE_ARGS}" <<'REMOTE_DAEMON'
BASE="$1"
TYPE="$2"
shift 2
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

# Clean up any leftover screen session
screen -X -S dbsrt_batch quit 2>/dev/null || true

# Lifecycle markers read by batch-status: batch type to resume, no manual stop pending
echo "${TYPE}" > run/batch_type
rm -f run/batch_stopped run/batch_aborted run/batch_queue.json

export PATH="$HOME/${BASE}/bin:$PATH"
export RAYON_NUM_THREADS=2
ARGS=$(printf '%q ' "$@")

# Launch in persistent detached screen session immune to SSH session termination
screen -dmS dbsrt_batch bash -c "echo \$\$ > '${PID_FILE}'; exec ./bin/python3 -u scripts/o2switch_batch_worker.py ${ARGS} >> '${LOG_FILE}' 2>&1"
sleep 1
PID=$(cat "${PID_FILE}" 2>/dev/null || true)
echo "✓ Started persistent background batch worker (screen: dbsrt_batch, PID: ${PID})."
REMOTE_DAEMON
      echo "✓ Background batch ingestion worker started on ${O2SWITCH_HOST}."
      echo "  • Monitor status : task o2switch:batch:status"
      echo "  • Stream logs    : task o2switch:batch:logs"
      echo "  • Live dashboard : task o2switch:top"
      echo "  • Stop worker    : task o2switch:batch:stop"
    else
      echo "=== Executing ${TARGET_TYPE} batch ingestion on ${O2SWITCH_HOST} ==="
      # shellcheck disable=SC2029
      ssh -o ServerAliveInterval=15 -o ServerAliveCountMax=3 "${O2SWITCH_HOST}" "bash -s -- ${REMOTE_ARGS}" <<'REMOTE_FOREGROUND'
BASE="$1"
shift 2
cd "${BASE}"
export PATH="$HOME/${BASE}/bin:$PATH"
export RAYON_NUM_THREADS=2
exec ./bin/python3 -u scripts/o2switch_batch_worker.py "$@"
REMOTE_FOREGROUND
    fi
    ;;

  batch-status)
    check_ssh
    echo "=== Remote TVShow Batch Worker Status (${O2SWITCH_HOST}) ==="
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "bash -s -- '${REMOTE_BASE}'" <<'REMOTE_SCRIPT'
BASE="$1"
PID_FILE="${BASE}/run/batch_worker.pid"
LOG_FILE="${BASE}/logs/batch_worker.log"
QUEUE_FILE="${BASE}/run/batch_queue.json"
# Machine-readable lifecycle line parsed by rpi4_o2switch_supervisor.py:
#   running | stopped (manual batch-stop) | aborted (fast-failure streak, see run/batch_aborted)
#   | finished (queue drained) | crashed | absent (never started)
PID=""
STATE="absent"
if [[ -f "${PID_FILE}" ]]; then
  PID=$(cat "${PID_FILE}")
fi
if [[ -n "${PID}" ]] && kill -0 "${PID}" 2>/dev/null; then
  STATE="running"
elif [[ -f "${BASE}/run/batch_stopped" ]]; then
  STATE="stopped"
elif [[ -f "${BASE}/run/batch_aborted" ]]; then
  STATE="aborted"
  cat "${BASE}/run/batch_aborted"
elif [[ -f "${QUEUE_FILE}" ]] && grep -q '"remaining_targets": 0,' "${QUEUE_FILE}"; then
  STATE="finished"
elif [[ -n "${PID}" || -f "${QUEUE_FILE}" ]]; then
  STATE="crashed"
fi
echo "BATCH_STATE=${STATE} PID=${PID:-none} TYPE=$(cat "${BASE}/run/batch_type" 2>/dev/null || echo tvshow)"
if [[ "${STATE}" == "running" ]]; then
  echo "● Batch worker is RUNNING (PID: ${PID})"
  ps -o pid,user,%cpu,%mem,etime,comm -p "${PID}" || true
else
  echo "○ Batch worker is not running (${STATE})."
fi
if [[ -f "${QUEUE_FILE}" ]]; then
  echo ""
  echo "--- File de Traitement Batch (Progression) ---"
  python3 -c "
import json
try:
    with open('${QUEUE_FILE}') as f:
        d = json.load(f)
    print(f\"  • Progression file : {d.get('completed_targets', 0)} / {d.get('total_targets', 0)} traités ({d.get('progress_pct', 0.0):.1f}%)\")
    print(f\"  • En attente       : {d.get('remaining_targets', 0)} cibles restantes (Échecs: {d.get('failed_targets', 0)})\")
    print(f\"  • Concurrence      : {d.get('active_workers', 0)} workers parallèles\")
except Exception:
    pass
" 2>/dev/null || true
fi
if [[ -f "${LOG_FILE}" ]]; then
  echo ""
  echo "--- Recent Worker Activity (last 10 lines of ${LOG_FILE}) ---"
  tail -n 10 "${LOG_FILE}"
fi
REMOTE_SCRIPT
    ;;

  batch-stop)
    check_ssh
    echo "=== Stopping Batch Worker Daemon on ${O2SWITCH_HOST} ==="
    # Marker first, so the RPi4 supervisor does not heal a deliberate stop
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "touch $(printf '%q' "${REMOTE_BASE}/run/batch_stopped")"
    remote_stop_pidfile "run/batch_worker.pid"
    # The worker kills its dbsrt process groups on SIGTERM; this catches leftovers after a SIGKILL
    # shellcheck disable=SC2029
    ssh "${O2SWITCH_HOST}" "pkill -KILL -u \$(id -u) -f '[b]in/dbsrt media:add' || true; screen -X -S dbsrt_batch quit 2>/dev/null || true"
    ;;

  batch-workers)
    check_ssh
    shift || true
    NEW_COUNT="${1:-}"
    if [[ -n "${NEW_COUNT}" ]]; then
      if ! [[ "${NEW_COUNT}" =~ ^[0-9]+$ ]] || ((NEW_COUNT < MIN_BATCH_WORKERS || NEW_COUNT > MAX_BATCH_WORKERS)); then
        echo "❌ Worker count must be an integer in ${MIN_BATCH_WORKERS}-${MAX_BATCH_WORKERS} (got '${NEW_COUNT}')."
        exit 1
      fi
      echo "=== Setting remote batch concurrency to ${NEW_COUNT} workers on ${O2SWITCH_HOST} ==="
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "mkdir -p $(printf '%q' "${REMOTE_BASE}/run") && echo ${NEW_COUNT} > $(printf '%q' "${REMOTE_BASE}/run/desired_workers")"
      echo "✓ Set desired workers to ${NEW_COUNT}."
    else
      echo "=== Querying remote batch concurrency on ${O2SWITCH_HOST} ==="
      # shellcheck disable=SC2029
      ssh "${O2SWITCH_HOST}" "cat ${REMOTE_BASE}/run/desired_workers 2>/dev/null || echo 'unset (worker uses its --workers value)'"
    fi
    ;;

  batch-logs)
    check_ssh
    # shellcheck disable=SC2029
    ssh -t "${O2SWITCH_HOST}" "tail -f ${REMOTE_BASE}/logs/batch_worker.log"
    ;;

  faultwatch)
    # Per-minute LVE fault proxies, see docs/2026-10-31-o2switch-lve-cpu-faults-analysis.md
    check_ssh
    SUB="${2:-status}"
    case "${SUB}" in
      start)
        rsync -az "scripts/o2switch_fault_watch.py" "${O2SWITCH_HOST}:${REMOTE_BASE}/scripts/"
        # shellcheck disable=SC2029
        ssh "${O2SWITCH_HOST}" "cd $(printf '%q' "${REMOTE_BASE}") && mkdir -p run logs/faultwatch && if [ -f run/faultwatch.pid ] && kill -0 \$(cat run/faultwatch.pid) 2>/dev/null; then echo 'Fault watcher already running.'; else screen -X -S dbsrt_faultwatch quit 2>/dev/null; screen -dmS dbsrt_faultwatch bash -c 'echo \$\$ > run/faultwatch.pid; exec ./bin/python3 -u scripts/o2switch_fault_watch.py watch >> logs/faultwatch/watch.log 2>&1' && echo '✓ Fault watcher started (screen: dbsrt_faultwatch).'; fi"
        ;;
      stop)
        remote_stop_pidfile "run/faultwatch.pid"
        ;;
      status)
        # shellcheck disable=SC2029
        ssh "${O2SWITCH_HOST}" "cd $(printf '%q' "${REMOTE_BASE}") && if [ -f run/faultwatch.pid ] && kill -0 \$(cat run/faultwatch.pid) 2>/dev/null; then echo \"● Fault watcher RUNNING (PID \$(cat run/faultwatch.pid))\"; else echo '○ Fault watcher not running.'; fi; tail -n 3 logs/faultwatch/*.jsonl 2>/dev/null | tail -n 3; tail -n 5 logs/faultwatch/watch.log 2>/dev/null"
        ;;
      pull)
        mkdir -p data/reports/faultwatch
        rsync -az "${O2SWITCH_HOST}:${REMOTE_BASE}/logs/faultwatch/" data/reports/faultwatch/ --include='*.jsonl' --exclude='*'
        echo "✓ Pulled into data/reports/faultwatch/"
        ;;
      *)
        echo "Usage: $0 faultwatch {start|stop|status|pull}"
        exit 1
        ;;
    esac
    ;;

  *)
    echo "Usage: $0 {build|test-local|sync|start|stop|restart|status|logs|demo|setup-cron|remove-cron|rotate-key|rotate-readonly-key|monitoring|top|pull-catalog|backup-nas|batch-tvshow|batch-movies|batch-status|batch-workers|batch-stop|batch-logs|faultwatch}"
    exit 1
    ;;
esac
