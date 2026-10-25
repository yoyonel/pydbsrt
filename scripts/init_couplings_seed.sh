#!/usr/bin/env bash
# Copies the CI seed fixture to data/SRT/couplings.json if absent.
# Lightweight: no binary dependency, no PostgreSQL, no NAS scan.
set -euo pipefail

SUBTITLES_DIR="${SUBTITLES_DIR:-data/SRT}"
SEED_FIXTURE="${SEED_FIXTURE:-tests/fixtures/couplings.seed.json}"

if [[ ! -f "${SUBTITLES_DIR}/couplings.json" ]]; then
  if [[ -f "$SEED_FIXTURE" ]]; then
    mkdir -p "${SUBTITLES_DIR}"
    cp "$SEED_FIXTURE" "${SUBTITLES_DIR}/couplings.json"
    echo "✓ Initialized ${SUBTITLES_DIR}/couplings.json from ${SEED_FIXTURE}"
  else
    echo "⚠ Seed fixture not found at ${SEED_FIXTURE}, skipping init." >&2
  fi
fi
