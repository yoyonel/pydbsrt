#!/usr/bin/env bash
# Self-contained Playwright E2E runner for pydbsrt Python MediaSources Portal (:8780).
set -euo pipefail

DEFAULT_PORTAL_PORT=8780
READY_POLL_RETRIES=30
READY_POLL_SLEEP_SECS=0.2

PORTAL_PORT="${DBSRT_PORTAL_PORT:-${DEFAULT_PORTAL_PORT}}"
BASE_URL="http://127.0.0.1:${PORTAL_PORT}"
EXPLORER_ONLY=false

for arg in "$@"; do
  case "${arg}" in
    --explorer-only)
      EXPLORER_ONLY=true
      ;;
  esac
done

STARTED_PID=""
cleanup() {
  if [[ -n "${STARTED_PID}" ]]; then
    kill "${STARTED_PID}" 2>/dev/null || true
    wait "${STARTED_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ! curl -sf "${BASE_URL}/api/sources" >/dev/null 2>&1; then
  echo "[E2E] Booting temporary pydbsrt.portal daemon on ${BASE_URL}..."
  uv run python -m pydbsrt.portal --port "${PORTAL_PORT}" >/tmp/dbsrt_sources_e2e.log 2>&1 &
  STARTED_PID=$!

  ready=false
  for _ in $(seq 1 "${READY_POLL_RETRIES}"); do
    if curl -sf "${BASE_URL}/api/sources" >/dev/null 2>&1; then
      ready=true
      break
    fi
    sleep "${READY_POLL_SLEEP_SECS}"
  done

  if [[ "${ready}" != "true" ]]; then
    echo "[E2E] ERROR: pydbsrt.portal failed to become ready on ${BASE_URL}" >&2
    cat /tmp/dbsrt_sources_e2e.log >&2 || true
    exit 1
  fi
else
  echo "[E2E] Reusing running pydbsrt.portal daemon on ${BASE_URL}..."
fi

if [[ "${EXPLORER_ONLY}" == "true" ]]; then
  uv run --with playwright python scripts/e2e/e2e_sources_explorer.py --url "${BASE_URL}/sources.html"
else
  uv run --with playwright python scripts/e2e/e2e_sources_explorer.py --url "${BASE_URL}/sources.html"
  uv run --with playwright python scripts/e2e/e2e_trajectories_formal.py --url "${BASE_URL}/#view=trajectories"
  uv run --with playwright python scripts/e2e/e2e_portal_exhaustive.py --url "${BASE_URL}/sources.html"
fi
