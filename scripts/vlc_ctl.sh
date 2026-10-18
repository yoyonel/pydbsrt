#!/usr/bin/env bash
# Helper script for VLC Lua plugin installation and headless cvlc E2E validation.
set -euo pipefail

ACTION="${1:-install}"

install_plugin() {
  mkdir -p ~/.local/share/vlc/lua/extensions/ ~/.local/share/vlc/lua/intf/
  cp -v vlc-plugin/vlsub_dbsrt.lua ~/.local/share/vlc/lua/extensions/
  cp -v vlc-plugin/dbsrt_subsync.lua ~/.local/share/vlc/lua/extensions/
  cp -v vlc-plugin/dbsrt_intf.lua ~/.local/share/vlc/lua/intf/
  echo "dbsrt VLC plugins installed (VLSub dbsrt + subsync in extensions/, CLI in intf/)."
}

test_cvlc() {
  local output
  output="$(DBSRT_AUTOKILL=1 cvlc --avcodec-hw=none --extraintf luaintf --lua-intf dbsrt_intf tests/data/big_buck_bunny_trailer_480p.mp4 2>&1)"
  echo "$output"
  if echo "$output" | grep -q "SUCCESS! Media matched"; then
    echo "✓ E2E VLC Integration Test Passed (Event-driven autokill)!"
  else
    echo "✗ E2E VLC Integration Test Failed: Subtitle was not attached!" >&2
    exit 1
  fi
}

case "$ACTION" in
  install) install_plugin ;;
  test-cvlc) test_cvlc ;;
  *)
    echo "Usage: $0 {install|test-cvlc}" >&2
    exit 1
    ;;
esac
