#!/usr/bin/env bash
# scripts/ingestion/media_add.sh: Thin orchestrator for dbsrt media:add CLI subcommand
# Strict adherence to Pure Taskfile Orchestrator (Zero inline logic)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel 2>/dev/null)"; then
  REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
fi
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

DBSRT_BIN="${DBSRT_BIN:-${REPO_ROOT}/target/release/dbsrt}"
if [[ ! -x "${DBSRT_BIN}" ]]; then
  if [[ -x "${REPO_ROOT}/target/debug/dbsrt" ]]; then
    DBSRT_BIN="${REPO_ROOT}/target/debug/dbsrt"
  else
    echo "❌ Error: dbsrt binary not found at ${DBSRT_BIN} (run 'task cli:build' or 'task rs:build' first)." >&2
    exit 1
  fi
fi

exec "${DBSRT_BIN}" media:add "$@"
