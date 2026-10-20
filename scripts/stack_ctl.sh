#!/usr/bin/env bash
# Unified daemon and health controller for dbsrt-server, MkDocs, PgWeb, Prometheus, and Grafana.
set -euo pipefail

ACTION="${1:-status}"
SERVER_BIN="${SERVER_BIN:-./target/release/dbsrt-server}"
SERVER_PID_FILE="/tmp/dbsrt-server.pid"
SERVER_LOG_FILE="/tmp/dbsrt-server.log"
SERVER_DISCOVERY_FILE="/tmp/dbsrt-server.json"

DOCS_PID_FILE="/tmp/dbsrt-docs.pid"
DOCS_LOG_FILE="/tmp/dbsrt-docs.log"

SERVER_HOST="${SERVER_HOST:-127.0.0.1}"
DEFAULT_SERVER_PORT=8765
SERVER_PORT="${SERVER_PORT:-$DEFAULT_SERVER_PORT}"
DEFAULT_PGWEB_PORT=8086
PGWEB_PORT="${PGWEB_PORT:-$DEFAULT_PGWEB_PORT}"
DEFAULT_DOCS_PORT=8000
DOCS_PORT="${DOCS_PORT:-$DEFAULT_DOCS_PORT}"
DEFAULT_POSTGRES_PORT=5433
POSTGRES_PORT="${PSQL_PORT:-$DEFAULT_POSTGRES_PORT}"
DEFAULT_EXPORTER_PORT=9187
EXPORTER_PORT="${EXPORTER_PORT:-$DEFAULT_EXPORTER_PORT}"
DEFAULT_PROMETHEUS_PORT=9090
PROMETHEUS_PORT="${PROMETHEUS_PORT:-$DEFAULT_PROMETHEUS_PORT}"
DEFAULT_GRAFANA_PORT=3000
GRAFANA_PORT="${GRAFANA_PORT:-$DEFAULT_GRAFANA_PORT}"

DEFAULT_START_WAIT_SECONDS="0.5"
DEFAULT_STOP_WAIT_SECONDS="0.3"
DEFAULT_DOCS_WAIT_SECONDS="1"

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  COLOR_RESET="\033[0m"
  COLOR_BOLD="\033[1m"
  COLOR_GREEN="\033[0;32m"
  COLOR_CYAN="\033[0;36m"
  COLOR_YELLOW="\033[1;33m"
  COLOR_RED="\033[0;31m"
else
  COLOR_RESET=""
  COLOR_BOLD=""
  COLOR_GREEN=""
  COLOR_CYAN=""
  COLOR_YELLOW=""
  COLOR_RED=""
fi

check_http_service() {
  local label="$1"
  local url="$2"
  local display_url="${3:-$url}"
  local http_code
  http_code="$(curl -s -o /dev/null -w "%{http_code}" "$url" 2>/dev/null || echo "000")"
  if [[ "$http_code" =~ ^(200|301|302)$ ]]; then
    printf "  %-32s : %b[ UP ] HTTP %s%b (%s)\n" "$label" "${COLOR_GREEN}" "$http_code" "${COLOR_RESET}" "$display_url"
  else
    printf "  %-32s : %b[DOWN] HTTP %s%b (%s)\n" "$label" "${COLOR_RED}" "$http_code" "${COLOR_RESET}" "$display_url"
  fi
}

check_tcp_service() {
  local label="$1"
  local host="$2"
  local port="$3"
  local desc="$4"
  if (echo >"/dev/tcp/${host}/${port}") >/dev/null 2>&1; then
    printf "  %-32s : %b[ UP ]%b (%s:%s - %s)\n" "$label" "${COLOR_GREEN}" "${COLOR_RESET}" "$host" "$port" "$desc"
  else
    printf "  %-32s : %b[DOWN]%b (%s:%s - %s)\n" "$label" "${COLOR_RED}" "${COLOR_RESET}" "$host" "$port" "$desc"
  fi
}

check_vlc_plugins() {
  local vlc_ext="${HOME}/.local/share/vlc/lua/extensions/vlsub_dbsrt.lua"
  local vlc_intf="${HOME}/.local/share/vlc/lua/intf/dbsrt_intf.lua"
  if [[ -f "$vlc_ext" && -f "$vlc_intf" ]]; then
    printf "  %-32s : %b[INSTALLED]%b (~/.local/share/vlc/lua/...)\n" "VLC Lua Plugins" "${COLOR_GREEN}" "${COLOR_RESET}"
  else
    printf "  %-32s : %b[NOT FOUND]%b (run 'task vlc:install-plugin')\n" "VLC Lua Plugins" "${COLOR_YELLOW}" "${COLOR_RESET}"
  fi
}

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
    --host "$SERVER_HOST" \
    --port "$SERVER_PORT" \
    --phash-dir /tmp \
    --extra-phash-dir data/__PHASH__ \
    --extra-phash-dir data/snippets \
    --subtitles-dir data/SRT \
    --db-host "$SERVER_HOST" \
    --db-port "$POSTGRES_PORT" \
    "${dbphash_args[@]}" \
    </dev/null >"$SERVER_LOG_FILE" 2>&1 &
  echo $! >"$SERVER_PID_FILE"
  sleep "$DEFAULT_START_WAIT_SECONDS"
  echo "dbsrt-server started in background (PID: $(cat "$SERVER_PID_FILE"))"
}

server_stop() {
  if [[ -f "$SERVER_PID_FILE" ]]; then
    kill "$(cat "$SERVER_PID_FILE")" 2>/dev/null || true
    rm -f "$SERVER_PID_FILE"
  fi
  pkill -f "target/release/dbsrt-server" 2>/dev/null || pkill -f "dbsrt-server" 2>/dev/null || true
  rm -f "$SERVER_DISCOVERY_FILE"
  sleep "$DEFAULT_STOP_WAIT_SECONDS"
  echo "dbsrt-server stopped."
}

server_status() {
  local port="$SERVER_PORT"
  if [[ -f "$SERVER_DISCOVERY_FILE" ]]; then
    port="$(grep -o '"port": *[0-9]*' "$SERVER_DISCOVERY_FILE" | grep -o '[0-9]*' || echo "$SERVER_PORT")"
  fi
  local res
  if res="$(curl -s -f "http://${SERVER_HOST}:${port}/v1/status" 2>/dev/null)"; then
    local uptime version phash_cnt coup_cnt pg_conn
    uptime="$(echo "$res" | jq -r '.uptime_seconds // 0')"
    version="$(echo "$res" | jq -r '.version // "unknown"')"
    phash_cnt="$(echo "$res" | jq -r '.indexed_phash_count // 0')"
    coup_cnt="$(echo "$res" | jq -r '.certified_couplings_count // 0')"
    pg_conn="$(echo "$res" | jq -r '.postgres_connected // false')"
    printf "✓ dbsrt-server is running on http://%s:%s (v%s, uptime: %ss, %s pHashes, %s couplings, Postgres: %s)\n" \
      "$SERVER_HOST" "$port" "$version" "$uptime" "$phash_cnt" "$coup_cnt" "$pg_conn"
  else
    echo "❌ dbsrt-server is offline (port ${port})" >&2
    exit 1
  fi
}

server_logs() {
  if [[ ! -f "$SERVER_LOG_FILE" ]]; then
    touch "$SERVER_LOG_FILE"
  fi
  tail -f "$SERVER_LOG_FILE"
}

doc_start() {
  if pgrep -f "mkdocs serve" >/dev/null 2>&1; then
    echo "MkDocs server is already running (PID: $(pgrep -f 'mkdocs serve' | head -n1))"
    exit 0
  fi
  setsid uv run --no-sync mkdocs serve --dev-addr "${SERVER_HOST}:${DOCS_PORT}" </dev/null >"$DOCS_LOG_FILE" 2>&1 &
  echo $! >"$DOCS_PID_FILE"
  sleep "$DEFAULT_DOCS_WAIT_SECONDS"
  echo "MkDocs documentation server started in background on http://${SERVER_HOST}:${DOCS_PORT} (PID: $(cat "$DOCS_PID_FILE"))"
}

doc_stop() {
  if [[ -f "$DOCS_PID_FILE" ]]; then
    kill "$(cat "$DOCS_PID_FILE")" 2>/dev/null || true
    rm -f "$DOCS_PID_FILE"
  fi
  pkill -f "mkdocs serve" 2>/dev/null || true
  echo "MkDocs server stopped."
}

doc_logs() {
  if [[ ! -f "$DOCS_LOG_FILE" ]]; then
    touch "$DOCS_LOG_FILE"
  fi
  tail -f "$DOCS_LOG_FILE"
}

stack_status() {
  echo -e "${COLOR_BOLD}================================================================================${COLOR_RESET}"
  echo -e "${COLOR_BOLD}                    pydbsrt — Unified Stack Status & Guide                      ${COLOR_RESET}"
  echo -e "${COLOR_BOLD}================================================================================${COLOR_RESET}"
  echo ""
  echo -e "${COLOR_BOLD}📡 SERVICES & HEALTH CHECK:${COLOR_RESET}"
  check_http_service "dbsrt Studio & REST API" "http://${SERVER_HOST}:${SERVER_PORT}/v1/status" "http://${SERVER_HOST}:${SERVER_PORT}/v1/status"
  check_http_service "MkDocs Portal & Dashboards" "http://${SERVER_HOST}:${DOCS_PORT}/" "http://${SERVER_HOST}:${DOCS_PORT}/"
  check_http_service "Grafana Dashboard (Docker)" "http://${SERVER_HOST}:${GRAFANA_PORT}/api/health" "http://${SERVER_HOST}:${GRAFANA_PORT}/"
  check_http_service "PgWeb SQL Explorer (Docker)" "http://${SERVER_HOST}:${PGWEB_PORT}/" "http://${SERVER_HOST}:${PGWEB_PORT}/"
  check_http_service "Prometheus TSDB (Docker)" "http://${SERVER_HOST}:${PROMETHEUS_PORT}/-/healthy" "http://${SERVER_HOST}:${PROMETHEUS_PORT}/"
  check_http_service "Prometheus Exporter (Docker)" "http://${SERVER_HOST}:${EXPORTER_PORT}/metrics" "http://${SERVER_HOST}:${EXPORTER_PORT}/metrics"
  check_tcp_service "PostgreSQL 17 SP-GiST" "${SERVER_HOST}" "${POSTGRES_PORT}" "bktreedb"
  check_vlc_plugins

  echo ""
  echo -e "${COLOR_BOLD}🗄️  DATABASE & FINGERPRINT TELEMETRY:${COLOR_RESET}"
  local couplings_url="http://${SERVER_HOST}:${SERVER_PORT}/v1/couplings?summary=true"
  local data
  data="$(curl -s -f "$couplings_url" 2>/dev/null || echo "{}")"
  if [[ "$data" != "{}" ]]; then
    local pg_conn medias_cnt frames_cnt subs_cnt sub_frames_cnt phash_files cert_medias
    pg_conn="$(echo "$data" | jq -r '.postgres_connected // false')"
    medias_cnt="$(echo "$data" | jq -r '.postgres_counts.medias_count // 0')"
    frames_cnt="$(echo "$data" | jq -r '.postgres_counts.frames_count // 0')"
    subs_cnt="$(echo "$data" | jq -r '.postgres_counts.subtitles_count // 0')"
    sub_frames_cnt="$(echo "$data" | jq -r '.postgres_counts.sub_frames_count // 0')"
    phash_files="$(echo "$data" | jq -r '.indexed_phash_files // 0')"
    cert_medias="$(echo "$data" | jq -r '(.couplings | length) // 0')"

    printf "  • PostgreSQL 17 status : %s\n" "$([[ "$pg_conn" == "true" ]] && echo -e "${COLOR_GREEN}Connected${COLOR_RESET}" || echo -e "${COLOR_RED}Disconnected${COLOR_RESET}")"
    printf "  • Medias indexed in DB : %s\n" "$medias_cnt"
    printf "  • Video Frames (pHash) : %s\n" "$frames_cnt"
    printf "  • Subtitles registered : %s\n" "$subs_cnt"
    printf "  • Cue-Frame Anchors    : %s\n" "$sub_frames_cnt"
    printf "  • Local pHash Files    : %s\n" "$phash_files"
    printf "  • Certified Couplings  : %s\n" "$cert_medias"
  else
    printf "  %b(dbsrt-server telemetry unavailable)%b\n" "${COLOR_YELLOW}" "${COLOR_RESET}"
  fi

  echo ""
  echo -e "${COLOR_BOLD}🌐 USER INTERFACES & DASHBOARDS (Where to go):${COLOR_RESET}"
  printf "  %b🎛️  dbsrt Studio UI%b    -> %bhttp://%s:%s/studio%b\n" "${COLOR_CYAN}" "${COLOR_RESET}" "${COLOR_BOLD}" "${SERVER_HOST}" "${SERVER_PORT}" "${COLOR_RESET}"
  echo "      Application web : Timeline bi-piste, loupe 8x8 pHash, vidéo HTML5, recalage 1-clic."
  printf "  %b📊  Grafana Dashboard%b  -> %bhttp://%s:%s/%b\n" "${COLOR_CYAN}" "${COLOR_RESET}" "${COLOR_BOLD}" "${SERVER_HOST}" "${GRAFANA_PORT}" "${COLOR_RESET}"
  echo "      Monitoring visuel PostgreSQL 17 (QPS, connexions, cache hit ratio, deadlocks - zero login)."
  printf "  %b📚  Portail Doc & E2E%b  -> %bhttp://%s:%s/%b\n" "${COLOR_CYAN}" "${COLOR_RESET}" "${COLOR_BOLD}" "${SERVER_HOST}" "${DOCS_PORT}" "${COLOR_RESET}"
  echo "      Documentation, architecture Diátaxis, et Dashboard interactif E2E VLC NAS."
  printf "  %b🐘  PgWeb Explorer%b    -> %bhttp://%s:%s/%b\n" "${COLOR_CYAN}" "${COLOR_RESET}" "${COLOR_BOLD}" "${SERVER_HOST}" "${PGWEB_PORT}" "${COLOR_RESET}"
  echo "      Explorateur SQL zero-conf pour requêter medias, frames, subtitles et sub_frames."
  printf "  %b🔥  Prometheus Console%b -> %bhttp://%s:%s/%b\n" "${COLOR_CYAN}" "${COLOR_RESET}" "${COLOR_BOLD}" "${SERVER_HOST}" "${PROMETHEUS_PORT}" "${COLOR_RESET}"
  echo "      Moteur TSDB, exploration PromQL et cibles de scraping (/targets)."

  echo ""
  echo -e "${COLOR_BOLD}⚡ REST API ROUTES (dbsrt-server :${SERVER_PORT}):${COLOR_RESET}"
  echo "  GET  /v1/status                           Santé du démon, uptime, compteurs pHash et statut PG"
  echo "  GET  /v1/couplings?summary=true           Liste synthétique des couplages certifiés"
  echo "  GET  /v1/couplings/sub_frames             Pagination des ancres SRT (?media_stem=...&offset=0&limit=50)"
  echo "  GET  /v1/media/stream?path=<path>         Streaming vidéo partiel HTML5 (Range bytes)"
  echo "  POST /v1/search                           Recherche sous-titres (pHash consensus + local + OpenSubtitles)"
  echo "  POST /v1/certify                          Certification et insertion à chaud de couplage ground-truth"
  echo "  POST /v1/sync                             Recalage temporel direct et génération de .dbsrt.srt"

  echo ""
  echo -e "${COLOR_BOLD}📋 LOGS & DIAGNOSTICS:${COLOR_RESET}"
  echo "  • dbsrt-server logs  : task server:logs  (tail -f /tmp/dbsrt-server.log)"
  echo "  • MkDocs server logs : task doc:logs     (tail -f /tmp/dbsrt-docs.log)"
  echo "  • Docker stack logs  : task db:logs      (docker compose -f docker/docker-compose.yml logs -f)"

  echo ""
  echo -e "${COLOR_BOLD}🎬 VLC MEDIA PLAYER:${COLOR_RESET}"
  echo "  • task vlc:run       : Lance VLC avec l'extension VLSub dbsrt chargée"
  echo "  • task vlc:demo      : Démonstration live interactive avec sous-titre recalé et OSD"
  echo -e "${COLOR_BOLD}================================================================================${COLOR_RESET}"
}

case "$ACTION" in
  server-start) server_start ;;
  server-stop) server_stop ;;
  server-status) server_status ;;
  server-logs) server_logs ;;
  doc-start) doc_start ;;
  doc-stop) doc_stop ;;
  doc-logs) doc_logs ;;
  status) stack_status ;;
  *)
    echo "Usage: $0 {server-start|server-stop|server-status|server-logs|doc-start|doc-stop|doc-logs|status}" >&2
    exit 1
    ;;
esac
