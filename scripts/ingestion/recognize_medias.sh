#!/usr/bin/env bash
# Batch recognizes media files in parallel using pydbsrt app_cli recognize-media
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel 2>/dev/null)"; then
  REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
fi
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [--check | --dry-run | <media_root_dir>]"
  echo ""
  echo "Runs parallel media recognition across .mkv files using pydbsrt CLI."
  echo ""
  echo "Options:"
  echo "  --help, -h          Show this help message and exit"
  echo "  --check, --dry-run  Verify tools and scan directory without running recognition"
  echo ""
  echo "Environment variables:"
  echo "  MEDIA_ROOT_DIR      Target directory containing .mkv videos (default: \$HOME/media)"
  echo "  PARALLEL_NB_JOBS    Number of parallel worker jobs (default: 1)"
  echo "  OUTPUT_FORMAT       Output format: CSV, JSON, TEXT (default: CSV)"
  exit 0
fi

CLI_SCRIPT="${REPO_ROOT}/src/pydbsrt/app_cli.py"
if [[ ! -f "${CLI_SCRIPT}" ]]; then
  echo "❌ Error: CLI script not found at ${CLI_SCRIPT}" >&2
  exit 1
fi

DEFAULT_MEDIA_DIR="${HOME}/media"
PARALLEL_NB_JOBS="${PARALLEL_NB_JOBS:-1}"
OUTPUT_FORMAT="${OUTPUT_FORMAT:-CSV}"

if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  MEDIA_DIR="${MEDIA_ROOT_DIR:-${DEFAULT_MEDIA_DIR}}"
  echo "REPO_ROOT: ${REPO_ROOT}"
  echo "CLI_SCRIPT: ${CLI_SCRIPT} (exists: yes)"
  echo "MEDIA_DIR: ${MEDIA_DIR} (exists: $(test -d "${MEDIA_DIR}" && echo "yes" || echo "no"))"
  echo "PARALLEL_NB_JOBS: ${PARALLEL_NB_JOBS}"
  echo "OUTPUT_FORMAT: ${OUTPUT_FORMAT}"
  echo "parallel tool: $(command -v parallel >/dev/null 2>&1 && echo "installed" || echo "missing")"
  echo "uv tool: $(command -v uv >/dev/null 2>&1 && echo "installed" || echo "missing")"
  exit 0
fi

MEDIA_DIR="${1:-${MEDIA_ROOT_DIR:-${DEFAULT_MEDIA_DIR}}}"

if [[ ! -d "${MEDIA_DIR}" ]]; then
  echo "❌ Error: Media directory not found at ${MEDIA_DIR}" >&2
  exit 1
fi

if ! command -v parallel >/dev/null 2>&1; then
  echo "❌ Error: 'parallel' command not found. Install moreutils or GNU parallel." >&2
  exit 1
fi

TIME_CMD=""
if [[ -x "/usr/bin/time" ]]; then
  TIME_CMD="/usr/bin/time -v"
fi

# Pipeline: find files -> parallel recognize-media
find "${MEDIA_DIR}" -type f -name "*.mkv" |
  ${TIME_CMD} \
    parallel -j"${PARALLEL_NB_JOBS}" --eta -I% --max-args 1 --tty \
    uv run python "${CLI_SCRIPT}" recognize-media -m % --output_format "${OUTPUT_FORMAT}"
