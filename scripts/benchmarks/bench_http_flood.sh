#!/usr/bin/env bash
# Orchestrates end-to-end HTTP matching flood benchmark against local dbsrt-server
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [options] [-- <bench_http_flood options>]"
  echo ""
  echo "Orchestrates end-to-end HTTP matching flood benchmark against local dbsrt-server."
  echo ""
  echo "Environment variables:"
  echo "  BENCH_PORT    Port to bind dbsrt-server (default: 8769)"
  exit 0
fi

# Named operational constants
DEFAULT_BENCH_PORT=8769
BENCH_PORT="${BENCH_PORT:-${DEFAULT_BENCH_PORT}}"
BENCH_HOST="127.0.0.1"
TARGET_URL="http://${BENCH_HOST}:${BENCH_PORT}/v1/recognize"
CATALOG_PATH="${REPO_ROOT}/data/catalog.dbphash"
SERVER_BIN="${REPO_ROOT}/target/release/dbsrt-server"
DEFAULT_START_WAIT_SECONDS=0.5
MAX_HEALTHCHECK_ATTEMPTS=30
PID_FILE=$(mktemp)
LOG_FILE=$(mktemp)

if [[ ! -f "${CATALOG_PATH}" ]]; then
  echo "❌ Error: Catalog not found at ${CATALOG_PATH}. Run 'task o2switch:pull-catalog' first." >&2
  exit 1
fi

echo "=== DBSrt Server: End-to-End HTTP Matching Flood Harness ==="

# 1. Build release binaries
if [[ ! -f "${SERVER_BIN}" ]]; then
  echo "-> Compiling dbsrt-server in release mode..."
  cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" -p dbsrt-server
fi

# 2. Cleanup hook
cleanup() {
  if [[ -f "${PID_FILE}" ]]; then
    local s_pid
    s_pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
    if [[ -n "${s_pid}" ]] && kill -0 "${s_pid}" 2>/dev/null; then
      echo "-> Stopping background dbsrt-server (PID: ${s_pid})..."
      kill "${s_pid}" 2>/dev/null || true
      wait "${s_pid}" 2>/dev/null || true
    fi
    rm -f "${PID_FILE}"
  fi
  rm -f "${LOG_FILE}"
}
trap cleanup EXIT INT TERM

# 3. Start dedicated dbsrt-server
echo "-> Starting dbsrt-server on ${BENCH_HOST}:${BENCH_PORT} with ${CATALOG_PATH}..."
"${SERVER_BIN}" \
  --host "${BENCH_HOST}" \
  --port "${BENCH_PORT}" \
  --dbphash "${CATALOG_PATH}" \
  --subtitles-dir "${REPO_ROOT}/data/SRT" \
  --phash-dir /tmp \
  >"${LOG_FILE}" 2>&1 &
SERVER_PID=$!
echo "${SERVER_PID}" >"${PID_FILE}"

# 4. Wait for healthcheck
echo "-> Waiting for service readiness on http://${BENCH_HOST}:${BENCH_PORT}/v1/health..."
HEALTHY=false
for _ in $(seq 1 "${MAX_HEALTHCHECK_ATTEMPTS}"); do
  if curl -s -f "http://${BENCH_HOST}:${BENCH_PORT}/v1/health" >/dev/null 2>&1; then
    HEALTHY=true
    break
  fi
  sleep "${DEFAULT_START_WAIT_SECONDS}"
done

if [[ "${HEALTHY}" != "true" ]]; then
  echo "❌ Error: dbsrt-server failed to become ready. Log output:" >&2
  cat "${LOG_FILE}" >&2
  exit 1
fi
echo "✓ dbsrt-server is UP and healthy."

# 5. Run benchmark
cargo run --release --manifest-path "${REPO_ROOT}/Cargo.toml" -p dbsrt-server --example bench_http_flood -- --url "${TARGET_URL}" "$@"
