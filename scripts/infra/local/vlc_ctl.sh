#!/usr/bin/env bash
# Helper script for VLC Lua plugin installation and headless cvlc E2E validation.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || "${1:-}" == "help" ]]; then
  echo "Usage: $(basename "$0") {install|test-cvlc|run|demo} [media_path] [--help]"
  echo ""
  echo "Controls VLC Lua plugin installation and headless cvlc E2E validation."
  echo ""
  echo "Actions:"
  echo "  install      Install VLC Lua plugins (~/.local/share/vlc/lua/...) (default)"
  echo "  test-cvlc    Run headless cvlc integration test with event-driven autokill"
  echo "  run [media]  Launch VLC with official plugins and target media"
  echo "  demo [media] Launch VLC interactive demonstration"
  exit 0
fi

ACTION="${1:-install}"
DEFAULT_MEDIA="${REPO_ROOT}/tests/data/big_buck_bunny_trailer_480p.mp4"

install_plugin() {
  mkdir -p ~/.local/share/vlc/lua/extensions/ ~/.local/share/vlc/lua/intf/
  # Clean deprecated/redundant plugins
  rm -f ~/.local/share/vlc/lua/extensions/dbsrt_subsync.lua
  # Install official studio extension and CLI interface
  cp -v "${REPO_ROOT}/vlc-plugin/vlsub_dbsrt.lua" ~/.local/share/vlc/lua/extensions/
  cp -v "${REPO_ROOT}/vlc-plugin/dbsrt_intf.lua" ~/.local/share/vlc/lua/intf/
  echo "dbsrt VLC plugins installed (VLSub dbsrt in extensions/, CLI in intf/)."
}

test_cvlc() {
  local mode="${DBSRT_SEARCH_MODE:-auto}"
  local media="${1:-${DEFAULT_MEDIA}}"
  local output=""
  local rc=0
  output="$(DBSRT_AUTOKILL=1 DBSRT_SEARCH_MODE="${mode}" cvlc --vout=dummy --aout=dummy --no-audio --avcodec-hw=none --extraintf luaintf --lua-intf dbsrt_intf "${media}" 2>&1)" || rc=$?
  echo "$output"
  if echo "$output" | grep -q "SUCCESS! Media matched"; then
    echo "✓ E2E VLC Integration Test Passed (Event-driven autokill, mode: ${mode}, rc: ${rc})!"
  else
    echo "✗ E2E VLC Integration Test Failed (rc: ${rc}): Subtitle was not attached!" >&2
    exit 1
  fi
}

test_cvlc_ahash() {
  DBSRT_SEARCH_MODE="ahash" test_cvlc "$@"
}

run_vlc() {
  install_plugin
  local media="${2:-${DEFAULT_MEDIA}}"
  vlc --no-sub-autodetect-file --avcodec-hw=none "$media"
}

demo_vlc() {
  install_plugin
  local media="${2:-${DEFAULT_MEDIA}}"
  echo "Lancement VLC pour démonstration interactive dbsrt..."
  echo "Dans VLC : Menu Vue -> VLSub (dbsrt) -> Clic sur 'Recherche par pHash'"
  vlc --no-sub-autodetect-file --avcodec-hw=none "$media"
}

case "$ACTION" in
  install) install_plugin ;;
  test-cvlc)
    shift
    test_cvlc "$@"
    ;;
  test-cvlc-ahash)
    shift
    test_cvlc_ahash "$@"
    ;;
  run) run_vlc "$@" ;;
  demo) demo_vlc "$@" ;;
  *)
    echo "Usage: $(basename "$0") {install|test-cvlc|run|demo} [media_path]" >&2
    exit 1
    ;;
esac
