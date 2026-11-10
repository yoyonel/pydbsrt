#!/usr/bin/env bash
# Dual-process Distributed Dev Stack Controller
# Simulates the distributed topology: Local Agent (8765) <-> Remote Server (8766, o2switch/NAS/RPi4+ simulator)
set -euo pipefail

ACTION="${1:-status}"
SERVER_BIN="${SERVER_BIN:-./target/release/dbsrt-server}"

AGENT_PID_FILE="/tmp/dbsrt-agent.pid"
AGENT_LOG_FILE="/tmp/dbsrt-agent.log"
AGENT_PORT=8765

REMOTE_PID_FILE="/tmp/dbsrt-remote-server.pid"
REMOTE_LOG_FILE="/tmp/dbsrt-remote-server.log"
REMOTE_PORT=8766

ensure_binary() {
  if [[ ! -x "$SERVER_BIN" ]]; then
    echo "Building release binary $SERVER_BIN..."
    cargo build --release --bin dbsrt-server
  fi
}

start_distributed() {
  ensure_binary

  if [[ -f "$REMOTE_PID_FILE" ]] && kill -0 "$(cat "$REMOTE_PID_FILE")" 2>/dev/null; then
    echo "Simulated remote server already running (PID: $(cat "$REMOTE_PID_FILE")) on port $REMOTE_PORT"
  else
    local dbphash_args=()
    if [[ -f "data/catalog.dbphash" ]]; then
      dbphash_args=(--dbphash data/catalog.dbphash)
    fi

    echo "1. Starting Simulated Remote Server on http://127.0.0.1:$REMOTE_PORT..."
    setsid "$SERVER_BIN" \
      --host 127.0.0.1 \
      --port "$REMOTE_PORT" \
      --phash-dir data/__PHASH__ \
      --extra-phash-dir data/snippets \
      --subtitles-dir data/SRT \
      "${dbphash_args[@]}" \
      </dev/null >"$REMOTE_LOG_FILE" 2>&1 &
    echo $! >"$REMOTE_PID_FILE"

    # Wait for remote server health
    local ready=0
    for _ in {1..30}; do
      if curl -s -f "http://127.0.0.1:$REMOTE_PORT/v1/health" >/dev/null 2>&1; then
        ready=1
        break
      fi
      sleep 0.1
    done
    if [[ $ready -eq 1 ]]; then
      echo "   ✓ Remote Server active (PID: $(cat "$REMOTE_PID_FILE"))"
    else
      echo "   ✗ Failed to start Remote Server. Check $REMOTE_LOG_FILE"
      exit 1
    fi
  fi

  if [[ -f "$AGENT_PID_FILE" ]] && kill -0 "$(cat "$AGENT_PID_FILE")" 2>/dev/null; then
    echo "Local Agent already running (PID: $(cat "$AGENT_PID_FILE")) on port $AGENT_PORT"
  else
    echo "2. Starting Local Agent on http://127.0.0.1:$AGENT_PORT relaying to http://127.0.0.1:$REMOTE_PORT..."
    setsid "$SERVER_BIN" \
      --host 127.0.0.1 \
      --port "$AGENT_PORT" \
      --agent \
      --remote-api "http://127.0.0.1:$REMOTE_PORT" \
      --phash-dir /tmp \
      --subtitles-dir /tmp \
      </dev/null >"$AGENT_LOG_FILE" 2>&1 &
    echo $! >"$AGENT_PID_FILE"

    local agent_ready=0
    for _ in {1..30}; do
      if curl -s -f "http://127.0.0.1:$AGENT_PORT/v1/health" >/dev/null 2>&1; then
        agent_ready=1
        break
      fi
      sleep 0.1
    done
    if [[ $agent_ready -eq 1 ]]; then
      echo "   ✓ Local Agent active (PID: $(cat "$AGENT_PID_FILE"))"
    else
      echo "   ✗ Failed to start Local Agent. Check $AGENT_LOG_FILE"
      exit 1
    fi
  fi

  echo "================================================================="
  echo " ✓ Distributed Dev Stack is ready!"
  echo "   * VLC Plugin Endpoint : http://127.0.0.1:$AGENT_PORT"
  echo "   * Remote Cloud Target : http://127.0.0.1:$REMOTE_PORT"
  echo "   * Agent Metrics       : http://127.0.0.1:$AGENT_PORT/v1/metrics"
  echo "   * Remote Metrics      : http://127.0.0.1:$REMOTE_PORT/v1/metrics"
  echo "================================================================="
}

stop_distributed() {
  echo "Stopping Distributed Dev Stack..."
  if [[ -f "$AGENT_PID_FILE" ]]; then
    kill "$(cat "$AGENT_PID_FILE")" 2>/dev/null || true
    rm -f "$AGENT_PID_FILE"
  fi
  if [[ -f "$REMOTE_PID_FILE" ]]; then
    kill "$(cat "$REMOTE_PID_FILE")" 2>/dev/null || true
    rm -f "$REMOTE_PID_FILE"
  fi

  # Fallback kill on known ports
  fuser -k "${AGENT_PORT}/tcp" 2>/dev/null || true
  fuser -k "${REMOTE_PORT}/tcp" 2>/dev/null || true

  rm -f /tmp/dbsrt-server.json
  echo "✓ Distributed Dev Stack stopped."
}

status_distributed() {
  echo "================================================================="
  echo "       pydbsrt — Distributed Dev Stack (Local <-> Remote)        "
  echo "================================================================="

  echo -n "1. Local Agent (Port $AGENT_PORT)  : "
  if curl -s -f "http://127.0.0.1:$AGENT_PORT/v1/health" >/dev/null 2>&1; then
    echo "ONLINE"
    if command -v jq >/dev/null 2>&1; then
      echo "   Metrics: $(curl -s "http://127.0.0.1:$AGENT_PORT/v1/metrics" | jq -c '{total_requests, search_requests, remote_errors, avg_latency_ms}')"
    fi
  else
    echo "OFFLINE"
  fi

  echo -n "2. Remote Server (Port $REMOTE_PORT): "
  if curl -s -f "http://127.0.0.1:$REMOTE_PORT/v1/health" >/dev/null 2>&1; then
    echo "ONLINE (Simulating o2switch/NAS/RPi4+)"
    if command -v jq >/dev/null 2>&1; then
      echo "   Metrics: $(curl -s "http://127.0.0.1:$REMOTE_PORT/v1/metrics" | jq -c '{total_requests, recognize_requests, avg_latency_ms}')"
    fi
  else
    echo "OFFLINE"
  fi
  echo "================================================================="
}

case "$ACTION" in
  start | up) start_distributed ;;
  stop | down) stop_distributed ;;
  status) status_distributed ;;
  *)
    echo "Usage: $0 {start|stop|status}"
    exit 1
    ;;
esac
