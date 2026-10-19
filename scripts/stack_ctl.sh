#!/usr/bin/env bash
# Unified daemon and health controller for dbsrt-server, MkDocs, and PgWeb.
set -euo pipefail

ACTION="${1:-status}"
SERVER_BIN="${SERVER_BIN:-./target/release/dbsrt-server}"
SERVER_PID_FILE="/tmp/dbsrt-server.pid"
SERVER_LOG_FILE="/tmp/dbsrt-server.log"
SERVER_DISCOVERY_FILE="/tmp/dbsrt-server.json"

DOCS_PID_FILE="/tmp/dbsrt-docs.pid"
DOCS_LOG_FILE="/tmp/dbsrt-docs.log"

server_start() {
  if pgrep -f "target/release/dbsrt-server" >/dev/null 2>&1; then
    echo "dbsrt-server is already running (PID: $(pgrep -f 'target/release/dbsrt-server' | head -n1))"
    exit 0
  fi
  local dbphash_args=()
  if [[ -f "data/catalog.dbphash" ]]; then
    dbphash_args=(--dbphash data/catalog.dbphash)
  fi

  setsid "$SERVER_BIN" \
    --host 127.0.0.1 \
    --port 8765 \
    --phash-dir /tmp \
    --extra-phash-dir data/__PHASH__ \
    --subtitles-dir data/SRT \
    --db-host 127.0.0.1 \
    --db-port 5433 \
    "${dbphash_args[@]}" \
    </dev/null >"$SERVER_LOG_FILE" 2>&1 &
  echo $! >"$SERVER_PID_FILE"
  sleep 0.5
  echo "dbsrt-server started in background (PID: $(cat "$SERVER_PID_FILE"))"
}

server_stop() {
  if [[ -f "$SERVER_PID_FILE" ]]; then
    kill "$(cat "$SERVER_PID_FILE")" 2>/dev/null || true
    rm -f "$SERVER_PID_FILE"
  fi
  pkill -f "target/release/dbsrt-server" 2>/dev/null || pkill -f "dbsrt-server" 2>/dev/null || true
  rm -f "$SERVER_DISCOVERY_FILE"
  sleep 0.3
  echo "dbsrt-server stopped."
}

server_status() {
  local port=8765
  if [[ -f "$SERVER_DISCOVERY_FILE" ]]; then
    port="$(grep -o '"port": *[0-9]*' "$SERVER_DISCOVERY_FILE" | grep -o '[0-9]*' || echo 8765)"
  fi
  if command -v jq >/dev/null 2>&1; then
    curl -s -f "http://127.0.0.1:${port}/v1/status" | jq .
  else
    curl -s -f "http://127.0.0.1:${port}/v1/status"
  fi
}

doc_start() {
  if pgrep -f "mkdocs serve" >/dev/null 2>&1; then
    echo "MkDocs server is already running (PID: $(pgrep -f 'mkdocs serve' | head -n1))"
    exit 0
  fi
  setsid uv run --no-sync mkdocs serve --dev-addr 127.0.0.1:8000 </dev/null >"$DOCS_LOG_FILE" 2>&1 &
  echo $! >"$DOCS_PID_FILE"
  sleep 1
  echo "MkDocs documentation server started in background on http://127.0.0.1:8000 (PID: $(cat "$DOCS_PID_FILE"))"
}

doc_stop() {
  if [[ -f "$DOCS_PID_FILE" ]]; then
    kill "$(cat "$DOCS_PID_FILE")" 2>/dev/null || true
    rm -f "$DOCS_PID_FILE"
  fi
  pkill -f "mkdocs serve" 2>/dev/null || true
  echo "MkDocs server stopped."
}

stack_status() {
  echo "================================================================="
  echo "                 pydbsrt — Unified Stack Status                  "
  echo "================================================================="
  curl -s -o /dev/null -w "1. dbsrt Studio UI   (http://127.0.0.1:8765/studio) : HTTP %{http_code}\n" http://127.0.0.1:8765/studio || true
  curl -s -o /dev/null -w "2. dbsrt REST API    (http://127.0.0.1:8765/v1/status): HTTP %{http_code}\n" http://127.0.0.1:8765/v1/status || true
  curl -s -o /dev/null -w "3. PgWeb Explorer    (http://127.0.0.1:8086/)       : HTTP %{http_code}\n" http://127.0.0.1:8086/ || true
  curl -s -o /dev/null -w "4. MkDocs Portal     (http://127.0.0.1:8000/)       : HTTP %{http_code}\n" http://127.0.0.1:8000/ || true
  echo "-----------------------------------------------------------------"
  curl -s "http://127.0.0.1:8765/v1/couplings?summary=true" |
    jq '{postgres_connected, postgres_counts, indexed_phash_files, certified_medias: (.couplings | length)}' 2>/dev/null ||
    echo "dbsrt-server offline"
  echo "================================================================="
}

case "$ACTION" in
  server-start) server_start ;;
  server-stop) server_stop ;;
  server-status) server_status ;;
  doc-start) doc_start ;;
  doc-stop) doc_stop ;;
  status) stack_status ;;
  *)
    echo "Usage: $0 {server-start|server-stop|server-status|doc-start|doc-stop|status}" >&2
    exit 1
    ;;
esac
