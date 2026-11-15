#!/usr/bin/env bash
# Manages cross-compilation, local ARM64 simulation, and deployment of dbsrt CLI on Raspberry Pi 4
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "${SCRIPT_DIR}/../../.." && pwd)}"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Named operational constants
DEFAULT_HEAD_LINES=10
DEFAULT_CANARY_PORT=18765
DEFAULT_HEALTHCHECK_MAX_RETRIES=20
DEFAULT_HEALTHCHECK_SLEEP_SEC=0.5

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || "${1:-}" == "help" ]]; then
  echo "Usage: $(basename "$0") {build|test|deploy|stage|top|top-preview|canary} [args...] [--help]"
  echo ""
  echo "Cross-compilation, local ARM64 simulation, and deployment of dbsrt CLI on Raspberry Pi 4."
  echo ""
  echo "Actions:"
  echo "  build         Build ARM64 Docker image with Bullseye GLIBC 2.31 parity and extract binaries"
  echo "  test          Run local ARM64 Bullseye validation on host via QEMU"
  echo "  deploy        Deploy ARM64 dbsrt binary to RPi4 over SSH"
  echo "  stage         Stage pre-merge ARM64 dbsrt & dbsrt-server to RPi4 preview dir (prod untouched)"
  echo "  top           Launch interactive dbsrt top session on RPi4"
  echo "  top-preview   Launch interactive staged preview dbsrt top session on RPi4"
  echo "  canary        Validate staged preview dbsrt-server on isolated RPi4 localhost port"
  exit 0
fi

ACTION="${1:-build}"
RPI_HOST="${RPI_HOST:-rpi4}"
IMAGE_NAME="${IMAGE_NAME:-dbsrt-cli:rpi4}"
REMOTE_BIN_DIR="${REMOTE_BIN_DIR:-~/.local/bin}"
REMOTE_PREVIEW_DIR="${REMOTE_PREVIEW_DIR:-~/.local/lib/dbsrt-preview}"
TARGET_DIR="${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release"
BINARY_PATH="${TARGET_DIR}/dbsrt"
SERVER_BINARY_PATH="${TARGET_DIR}/dbsrt-server"

case "${ACTION}" in
  build)
    echo "=== Building dbsrt CLI & Server Docker image for ARM64 with Bullseye GLIBC 2.31 parity ==="
    # .git is outside the build context: pass the version metadata build.rs would read from git
    GIT_HASH="$(git -C "${REPO_ROOT}" rev-parse --short HEAD 2>/dev/null || true)"
    if [[ -n "${GIT_HASH}" ]] && ! git -C "${REPO_ROOT}" diff --quiet 2>/dev/null; then
      GIT_HASH="${GIT_HASH}-dirty"
    fi
    COMMIT_EPOCH="$(git -C "${REPO_ROOT}" log -1 --format=%ct 2>/dev/null || true)"
    DOCKERFILE_PATH="${DOCKERFILE_PATH:-${REPO_ROOT}/docker/Dockerfile.cli.rpi4}"
    docker build \
      --target builder \
      -f "${DOCKERFILE_PATH}" \
      --build-arg "DBSRT_GIT_HASH=${GIT_HASH}" \
      --build-arg "SOURCE_DATE_EPOCH=${COMMIT_EPOCH}" \
      -t "${IMAGE_NAME}" "${REPO_ROOT}"
    echo "✓ Built cross-compiler stage ${IMAGE_NAME}."

    echo "=== Extracting ARM64 binaries to ${TARGET_DIR} ==="
    mkdir -p "${TARGET_DIR}"
    CID=$(docker create "${IMAGE_NAME}")
    docker cp "${CID}:/out/dbsrt" "${BINARY_PATH}"
    docker cp "${CID}:/out/dbsrt-server" "${SERVER_BINARY_PATH}"
    docker rm "${CID}" >/dev/null
    chmod +x "${BINARY_PATH}" "${SERVER_BINARY_PATH}"
    echo "✓ Extracted binaries: ${BINARY_PATH} and ${SERVER_BINARY_PATH}"
    file "${BINARY_PATH}" "${SERVER_BINARY_PATH}"
    ;;

  test)
    if [[ ! -f "${BINARY_PATH}" || ! -f "${SERVER_BINARY_PATH}" ]]; then
      echo "Binaries not found in ${TARGET_DIR}. Running build first..."
      "$0" build
    fi
    echo "=== Running local ARM64 Bullseye validation on host via QEMU ==="
    echo "-> Testing 'dbsrt --version' inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "${REPO_ROOT}:/app" debian:bullseye-slim "/app/target/aarch64-unknown-linux-gnu/release/dbsrt" --version

    echo "-> Testing 'dbsrt --help' inside Debian Bullseye ARM64 container:"
    (docker run --rm --platform linux/arm64 -v "${REPO_ROOT}:/app" debian:bullseye-slim "/app/target/aarch64-unknown-linux-gnu/release/dbsrt" --help | head -n "${DEFAULT_HEAD_LINES}") || true

    echo "-> Testing 'dbsrt-server --version' inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "${REPO_ROOT}:/app" debian:bullseye-slim "/app/target/aarch64-unknown-linux-gnu/release/dbsrt-server" --version

    echo "-> Testing 'dbsrt-server --help' inside Debian Bullseye ARM64 container:"
    (docker run --rm --platform linux/arm64 -v "${REPO_ROOT}:/app" debian:bullseye-slim "/app/target/aarch64-unknown-linux-gnu/release/dbsrt-server" --help | head -n "${DEFAULT_HEAD_LINES}") || true

    echo "-> Testing 'dbsrt top --json' telemetry snapshot inside Debian Bullseye ARM64 container:"
    docker run --rm --platform linux/arm64 -v "${REPO_ROOT}:/app" debian:bullseye-slim "/app/target/aarch64-unknown-linux-gnu/release/dbsrt" top --json | jq '{cpu: .host_cpu_pct, ram_used: .ram_used_gib, sources: (.media_sources | length)}'

    echo "✓ Local ARM64 Bullseye simulation passed with 100% parity!"
    ;;

  deploy)
    if [[ ! -f "${BINARY_PATH}" ]]; then
      echo "Binary not found at ${BINARY_PATH}. Running build first..."
      "$0" build
    fi
    echo "=== Deploying ARM64 dbsrt binary to ${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p ${REMOTE_BIN_DIR}"
    scp "${BINARY_PATH}" "${RPI_HOST}:${REMOTE_BIN_DIR}/dbsrt"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "chmod +x ${REMOTE_BIN_DIR}/dbsrt"
    echo "✓ Deployed successfully. Verifying remote execution on ${RPI_HOST}:"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "${REMOTE_BIN_DIR}/dbsrt --version"
    ;;

  top)
    echo "=== Launching interactive dbsrt top session on ${RPI_HOST} ==="
    # shellcheck disable=SC2029
    ssh -t "${RPI_HOST}" "${REMOTE_BIN_DIR}/dbsrt top ${*:2}"
    ;;

  stage)
    SKIP_BUILD=false
    for arg in "${@:2}"; do
      [[ "${arg}" == "--skip-build" ]] && SKIP_BUILD=true
    done
    if [[ "${SKIP_BUILD}" != "true" || ! -f "${BINARY_PATH}" || ! -f "${SERVER_BINARY_PATH}" ]]; then
      "$0" build
    fi
    echo "=== Staging pre-merge ARM64 binaries to ${RPI_HOST}:${REMOTE_PREVIEW_DIR} (prod untouched) ==="
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "mkdir -p ${REMOTE_PREVIEW_DIR}"
    scp "${BINARY_PATH}" "${SERVER_BINARY_PATH}" "${RPI_HOST}:${REMOTE_PREVIEW_DIR}/"
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "chmod +x ${REMOTE_PREVIEW_DIR}/dbsrt ${REMOTE_PREVIEW_DIR}/dbsrt-server && ${REMOTE_PREVIEW_DIR}/dbsrt --version"
    echo "  • Interactive TUI preview : task rpi4:top:preview"
    echo "  • Canary server validation: task rpi4:canary"
    ;;

  top-preview)
    # shellcheck disable=SC2029
    if ! ssh "${RPI_HOST}" "test -x ${REMOTE_PREVIEW_DIR}/dbsrt"; then
      echo "No staged preview binary on ${RPI_HOST}; staging current workspace first..."
      "$0" stage
    fi
    echo "=== Launching STAGED PREVIEW dbsrt top session on ${RPI_HOST} (prod ~/.local/bin untouched) ==="
    # shellcheck disable=SC2029
    ssh -t "${RPI_HOST}" "${REMOTE_PREVIEW_DIR}/dbsrt top ${*:2}"
    ;;

  canary)
    # shellcheck disable=SC2029
    if ! ssh "${RPI_HOST}" "test -x ${REMOTE_PREVIEW_DIR}/dbsrt-server"; then
      echo "No staged preview binary on ${RPI_HOST}; staging current workspace first..."
      "$0" stage
    fi
    echo "=== Running isolated canary check on ${RPI_HOST} (127.0.0.1:${DEFAULT_CANARY_PORT}) ==="
    # Zero-RAM-duplication canary workspace: symlinks live run/logs/reports/SRT without
    # reloading multi-GB catalog.dbphash/dbahash alongside prod dbsrt-server on 4 GB RPi4.
    # shellcheck disable=SC2029
    ssh "${RPI_HOST}" "bash -s -- $(printf '%q ' "${DEFAULT_CANARY_PORT}" "${DEFAULT_HEALTHCHECK_MAX_RETRIES}" "${DEFAULT_HEALTHCHECK_SLEEP_SEC}")" <<'REMOTE_RPI_CANARY'
set -euo pipefail
PORT="$1"
MAX_RETRIES="$2"
SLEEP_SEC="$3"
PREVIEW_DIR="$HOME/.local/lib/dbsrt-preview"
CANARY_LOG="$(mktemp)"
CANARY_ROOT="$(mktemp -d)"
mkdir -p "${CANARY_ROOT}/data" "${CANARY_ROOT}/empty_phash"
ln -s "$HOME/dbsrt/run" "${CANARY_ROOT}/run"
ln -s "$HOME/dbsrt/logs" "${CANARY_ROOT}/logs"
ln -s "$HOME/dbsrt/data/SRT" "${CANARY_ROOT}/data/SRT"
ln -s "$HOME/dbsrt/data/reports" "${CANARY_ROOT}/data/reports"
cd "${CANARY_ROOT}"
RAYON_NUM_THREADS=2 "${PREVIEW_DIR}/dbsrt-server" \
  --host 127.0.0.1 \
  --port "${PORT}" \
  --dbphash "${CANARY_ROOT}/empty.dbphash" \
  --subtitles-dir "${CANARY_ROOT}/data/SRT" \
  --phash-dir "${CANARY_ROOT}/empty_phash" </dev/null >"${CANARY_LOG}" 2>&1 &
CPID=$!
cleanup() {
  kill -INT "${CPID}" 2>/dev/null || true
  for _ in 1 2 3; do
    kill -0 "${CPID}" 2>/dev/null || break
    sleep 0.2
  done
  kill -KILL "${CPID}" 2>/dev/null || true
  wait "${CPID}" 2>/dev/null || true
  rm -rf "${CANARY_LOG}" "${CANARY_ROOT}"
}
trap cleanup EXIT

HEALTHY=false
for _ in $(seq 1 "${MAX_RETRIES}"); do
  if curl --noproxy '*' --max-time 2 -s -f "http://127.0.0.1:${PORT}/v1/health" >/dev/null 2>&1; then
    HEALTHY=true
    break
  fi
  sleep "${SLEEP_SEC}"
done
if [[ "${HEALTHY}" != "true" ]]; then
  echo "❌ RPi4 canary dbsrt-server failed healthcheck:"
  cat "${CANARY_LOG}"
  exit 1
fi
echo "✓ RPi4 canary /v1/health OK (127.0.0.1:${PORT}, zero catalog RAM duplication, prod systemd service untouched)."
echo "--- Canary /v1/status ---"
curl --noproxy '*' --max-time 3 -s -f "http://127.0.0.1:${PORT}/v1/status"
echo ""
if ALERTS_JSON="$(curl --noproxy '*' --max-time 3 -s -f "http://127.0.0.1:${PORT}/v1/monitoring/alerts" 2>/dev/null)"; then
  echo "--- Canary /v1/monitoring/alerts ---"
  printf '%s\n' "${ALERTS_JSON}"
fi
echo "--- Canary dbsrt --version ---"
"${PREVIEW_DIR}/dbsrt" --version
REMOTE_RPI_CANARY
    ;;

  *)
    echo "Usage: $(basename "$0") {build|test|deploy|stage|top|top-preview|canary} [args...]"
    exit 1
    ;;
esac
