#!/usr/bin/env bash
# scripts/web_top_ctl.sh: Orchestrates WebAssembly build and local serving for dbsrt-web-top (Ratzilla).
set -euo pipefail

DEFAULT_PORT=8085
DEFAULT_PUBLIC_URL="/dbsrt/top/"
DEFAULT_TRUNK_BIN="/home/linuxbrew/.linuxbrew/bin/trunk"
WEB_TOP_DIR="crates/dbsrt-web-top"

find_trunk() {
  if command -v trunk >/dev/null 2>&1; then
    command -v trunk
  elif [[ -x "${DEFAULT_TRUNK_BIN}" ]]; then
    echo "${DEFAULT_TRUNK_BIN}"
  else
    echo "❌ trunk CLI not found. Install via: cargo install --locked trunk (or brew install trunk)" >&2
    exit 1
  fi
}

TRUNK_BIN=$(find_trunk)

cmd_build() {
  echo "=== Building dbsrt-web-top (WebAssembly / Ratzilla) ==="
  local trunk_args=()
  local public_url="${PUBLIC_URL:-$DEFAULT_PUBLIC_URL}"
  for arg in "$@"; do
    case "${arg}" in
      --public-url=*)
        public_url="${arg#*=}"
        ;;
      *)
        trunk_args+=("${arg}")
        ;;
    esac
  done
  (cd "${WEB_TOP_DIR}" && "${TRUNK_BIN}" build --public-url "${public_url}" "${trunk_args[@]}")
  echo "✓ WebAssembly bundle generated in ${WEB_TOP_DIR}/dist"
}

cmd_serve() {
  local port="${PORT:-$DEFAULT_PORT}"
  echo "=== Serving dbsrt-web-top on http://127.0.0.1:${port} ==="
  (cd "${WEB_TOP_DIR}" && "${TRUNK_BIN}" serve --port "${port}")
}

usage() {
  echo "Usage: $0 {build|serve} [args...]"
  echo "  build [--release]  Compile WebAssembly bundle using trunk"
  echo "  serve              Serve WebAssembly dashboard locally on port ${DEFAULT_PORT}"
  exit 1
}

case "${1:-}" in
  build)
    shift
    cmd_build "$@"
    ;;
  serve)
    shift
    cmd_serve "$@"
    ;;
  *)
    usage
    ;;
esac
