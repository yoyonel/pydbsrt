#!/usr/bin/env bash
# Auto-discovers and certifies ground-truth (video/.phash <-> .srt) couplings
# into data/SRT/couplings.json and PostgreSQL (if reachable).
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
  echo "Usage: $(basename "$0") [--help] [--check | --dry-run] [-v | --verbose] [-q | --quiet]"
  echo ""
  echo "Auto-discovers and certifies ground-truth couplings into couplings.json and PostgreSQL."
  echo ""
  echo "Options:"
  echo "  --help, -h          Show this help message and exit"
  echo "  --check, --dry-run  Verify tools, paths and discoverable pairs without certifying"
  echo "  -v, --verbose       Print certification outputs"
  echo "  -q, --quiet         Suppress non-essential messages (default)"
  echo ""
  echo "Environment variables:"
  echo "  DBSRT_BIN           Path to dbsrt binary (default: target/release/dbsrt or debug)"
  echo "  PHASH_DIR           Directory for pHash files (default: <repo_root>/data/__PHASH__)"
  echo "  SUBTITLES_DIR       Directory for subtitles (default: <repo_root>/data/SRT)"
  echo "  PSQL_IP             PostgreSQL host (default: 127.0.0.1)"
  echo "  PSQL_PORT           PostgreSQL port (default: 5432)"
  exit 0
fi

DBSRT_BIN="${DBSRT_BIN:-${REPO_ROOT}/target/release/dbsrt}"
if [[ ! -x "${DBSRT_BIN}" ]]; then
  if [[ -x "${REPO_ROOT}/target/debug/dbsrt" ]]; then
    DBSRT_BIN="${REPO_ROOT}/target/debug/dbsrt"
  else
    echo "❌ Error: dbsrt binary not found at ${DBSRT_BIN} (run 'task cli:build' or 'task rs:build' first)." >&2
    exit 1
  fi
fi

PHASH_DIR="${PHASH_DIR:-${REPO_ROOT}/data/__PHASH__}"
SUBTITLES_DIR="${SUBTITLES_DIR:-${REPO_ROOT}/data/SRT}"
TMP_PHASH_DIR="${TMP_PHASH_DIR:-/tmp}"
DB_HOST="${PSQL_IP:-127.0.0.1}"
DEFAULT_DB_PORT=5432
DB_PORT="${PSQL_PORT:-$DEFAULT_DB_PORT}"

MAX_DB_RETRIES=10
DB_RETRY_INTERVAL_SECONDS="0.5"

if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  echo "REPO_ROOT: ${REPO_ROOT}"
  echo "DBSRT_BIN: ${DBSRT_BIN} (executable: yes)"
  echo "PHASH_DIR: ${PHASH_DIR} (exists: $(test -d "${PHASH_DIR}" && echo "yes" || echo "no"))"
  echo "SUBTITLES_DIR: ${SUBTITLES_DIR} (exists: $(test -d "${SUBTITLES_DIR}" && echo "yes" || echo "no"))"
  echo "DB target: ${DB_HOST}:${DB_PORT}"
  shopt -s nullglob
  srt_count=("${SUBTITLES_DIR}"/*.srt)
  echo "Discovered SRT files: ${#srt_count[@]}"
  exit 0
fi

QUIET=1
for arg in "$@"; do
  case "$arg" in
    -v | --verbose) QUIET=0 ;;
    -q | --quiet) QUIET=1 ;;
  esac
done

# Wait up to 5s for PostgreSQL to accept connections if container just started
for ((i = 1; i <= MAX_DB_RETRIES; i++)); do
  if (echo >"/dev/tcp/${DB_HOST}/${DB_PORT}") >/dev/null 2>&1; then
    break
  fi
  sleep "$DB_RETRY_INTERVAL_SECONDS"
done

# Initialize with clean CI seed fixture if registry is absent
SEED_FIXTURE="${REPO_ROOT}/tests/fixtures/couplings.seed.json"
if [[ ! -f "${SUBTITLES_DIR}/couplings.json" && -f "$SEED_FIXTURE" ]]; then
  mkdir -p "${SUBTITLES_DIR}"
  cp "$SEED_FIXTURE" "${SUBTITLES_DIR}/couplings.json"
fi

total_certified=0
total_errors=0

certify_media_pair() {
  local media_path="$1"
  local srt_path="$2"
  local lang="$3"

  local cmd=(
    "$DBSRT_BIN" certify-coupling
    --media "$media_path"
    --subtitle "$srt_path"
    --lang "$lang"
    --phash-dir "$TMP_PHASH_DIR"
    --subtitles-dir "$SUBTITLES_DIR"
    --db-host "$DB_HOST"
    --db-port "$DB_PORT"
  )
  if [[ "$QUIET" -eq 1 ]]; then
    cmd+=(--quiet)
  fi

  local output
  if output="$("${cmd[@]}" 2>&1)"; then
    if [[ "$QUIET" -eq 0 && -n "$output" ]]; then
      echo "$output"
    fi
    total_certified=$((total_certified + 1))
  else
    echo "❌ Error certifying media coupling '$media_path' ($lang):" >&2
    echo "$output" >&2
    total_errors=$((total_errors + 1))
  fi
}

certify_phash_pair() {
  local phash_path="$1"
  local srt_path="$2"
  local lang="$3"

  local cmd=(
    "$DBSRT_BIN" certify-coupling
    --phash-file "$phash_path"
    --subtitle "$srt_path"
    --lang "$lang"
    --subtitles-dir "$SUBTITLES_DIR"
    --db-host "$DB_HOST"
    --db-port "$DB_PORT"
    --allow-oob
  )
  if [[ "$QUIET" -eq 1 ]]; then
    cmd+=(--quiet)
  fi

  local output
  if output="$("${cmd[@]}" 2>&1)"; then
    if [[ "$QUIET" -eq 0 && -n "$output" ]]; then
      echo "$output"
    fi
    total_certified=$((total_certified + 1))
  else
    echo "❌ Error certifying pHash coupling '$phash_path' ($lang):" >&2
    echo "$output" >&2
    total_errors=$((total_errors + 1))
  fi
}

# 1. Local video test fixtures (Big Buck Bunny WebM + MP4)
for media in "${REPO_ROOT}/data/big_buck_bunny_trailer_480p.webm" "${REPO_ROOT}/tests/data/big_buck_bunny_trailer_480p.mp4"; do
  if [[ -f "$media" ]]; then
    for lang in fr en; do
      srt="${SUBTITLES_DIR}/big_buck_bunny_trailer_480p.${lang}.srt"
      [[ -f "$srt" ]] && certify_media_pair "$media" "$srt" "$lang"
    done
  fi
done

# 2. Auto-discover all (<release>.<lang>.srt <-> <release>.phash) pairs in data/SRT/ and data/__PHASH__/
shopt -s nullglob
for srt_path in "${SUBTITLES_DIR}"/*.srt; do
  fname="$(basename "$srt_path")"
  # Skip Big Buck Bunny already handled above as full video fixtures
  [[ "$fname" == big_buck_bunny_trailer_480p.* ]] && continue

  # Extract <stem>.<lang>.srt
  base_no_ext="${fname%.srt}"
  lang="${base_no_ext##*.}"
  stem="${base_no_ext%.*}"

  phash_candidate="${PHASH_DIR}/${stem}.phash"
  if [[ -f "$phash_candidate" ]]; then
    certify_phash_pair "$phash_candidate" "$srt_path" "$lang"
  fi
done

if [[ "$total_errors" -gt 0 ]]; then
  echo "❌ Coupling certification finished with ${total_errors} error(s) (${total_certified} succeeded)." >&2
  exit 1
else
  echo "✓ Certified ${total_certified} couplings into PostgreSQL & ${SUBTITLES_DIR}/couplings.json"
fi
