#!/usr/bin/env bash
# Copies the CI seed fixture to data/SRT/couplings.json if absent.
# Lightweight: no binary dependency, no PostgreSQL, no NAS scan.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [--check | --dry-run]"
  echo ""
  echo "Initializes data/SRT/couplings.json from tests/fixtures/couplings.seed.json if absent."
  echo ""
  echo "Options:"
  echo "  --help, -h          Show this help message and exit"
  echo "  --check, --dry-run  Verify source fixture existence and target status without copying"
  echo ""
  echo "Environment variables:"
  echo "  SUBTITLES_DIR       Directory for subtitles (default: <repo_root>/data/SRT)"
  echo "  SEED_FIXTURE        Source seed fixture file (default: <repo_root>/tests/fixtures/couplings.seed.json)"
  exit 0
fi

SUBTITLES_DIR="${SUBTITLES_DIR:-${REPO_ROOT}/data/SRT}"
SEED_FIXTURE="${SEED_FIXTURE:-${REPO_ROOT}/tests/fixtures/couplings.seed.json}"

if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  echo "REPO_ROOT: ${REPO_ROOT}"
  echo "SUBTITLES_DIR: ${SUBTITLES_DIR}"
  echo "SEED_FIXTURE: ${SEED_FIXTURE} (exists: $(test -f "${SEED_FIXTURE}" && echo "yes" || echo "no"))"
  echo "Target couplings.json exists: $(test -f "${SUBTITLES_DIR}/couplings.json" && echo "yes" || echo "no")"
  exit 0
fi

if [[ ! -f "${SUBTITLES_DIR}/couplings.json" ]]; then
  if [[ -f "${SEED_FIXTURE}" ]]; then
    mkdir -p "${SUBTITLES_DIR}"
    cp "${SEED_FIXTURE}" "${SUBTITLES_DIR}/couplings.json"
    echo "✓ Initialized ${SUBTITLES_DIR}/couplings.json from ${SEED_FIXTURE}"
  else
    echo "⚠ Seed fixture not found at ${SEED_FIXTURE}, skipping init." >&2
  fi
else
  echo "✓ Couplings file already present: ${SUBTITLES_DIR}/couplings.json"
fi
