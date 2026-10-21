#!/usr/bin/env bash
# Helper script for VLC Lua plugin installation and headless cvlc E2E validation.
set -euo pipefail

ACTION="${1:-install}"

install_plugin() {
  mkdir -p ~/.local/share/vlc/lua/extensions/ ~/.local/share/vlc/lua/intf/
  # Clean deprecated/redundant plugins
  rm -f ~/.local/share/vlc/lua/extensions/dbsrt_subsync.lua
  # Install official studio extension and CLI interface
  cp -v vlc-plugin/vlsub_dbsrt.lua ~/.local/share/vlc/lua/extensions/
  cp -v vlc-plugin/dbsrt_intf.lua ~/.local/share/vlc/lua/intf/
  echo "dbsrt VLC plugins installed (VLSub dbsrt in extensions/, CLI in intf/)."
}

test_cvlc() {
  local output
  output="$(DBSRT_AUTOKILL=1 cvlc --no-audio --avcodec-hw=none --extraintf luaintf --lua-intf dbsrt_intf tests/data/big_buck_bunny_trailer_480p.mp4 2>&1)"
  echo "$output"
  if echo "$output" | grep -q "SUCCESS! Media matched"; then
    echo "✓ E2E VLC Integration Test Passed (Event-driven autokill)!"
  else
    echo "✗ E2E VLC Integration Test Failed: Subtitle was not attached!" >&2
    exit 1
  fi
}

run_vlc() {
  install_plugin
  local media="${2:-tests/data/big_buck_bunny_trailer_480p.mp4}"
  vlc --no-sub-autodetect-file "$media"
}

demo_vlc() {
  install_plugin
  local media="${2:-tests/data/big_buck_bunny_trailer_480p.mp4}"
  echo "Lancement VLC pour démonstration interactive dbsrt..."
  echo "Dans VLC : Menu Vue -> VLSub (dbsrt) -> Clic sur 'Recherche par pHash'"
  vlc --no-sub-autodetect-file "$media"
}

case "$ACTION" in
  install) install_plugin ;;
  test-cvlc) test_cvlc ;;
  run) run_vlc "$@" ;;
  demo) demo_vlc "$@" ;;
  *)
    echo "Usage: $0 {install|test-cvlc|run|demo}" >&2
    exit 1
    ;;
esac
