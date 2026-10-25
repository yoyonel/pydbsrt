#!/usr/bin/env bash
# Auto-discovers and certifies ground-truth (video/.phash <-> .srt) couplings
# into data/SRT/couplings.json and PostgreSQL (if reachable).
set -euo pipefail

DBSRT_BIN="${DBSRT_BIN:-./target/release/dbsrt}"
PHASH_DIR="${PHASH_DIR:-data/__PHASH__}"
SUBTITLES_DIR="${SUBTITLES_DIR:-data/SRT}"
TMP_PHASH_DIR="${TMP_PHASH_DIR:-/tmp}"
DB_HOST="${PSQL_IP:-127.0.0.1}"
DEFAULT_DB_PORT=5433
DB_PORT="${PSQL_PORT:-$DEFAULT_DB_PORT}"

MAX_DB_RETRIES=10
DB_RETRY_INTERVAL_SECONDS="0.5"

QUIET=1
for arg in "$@"; do
  case "$arg" in
    -v | --verbose) QUIET=0 ;;
    -q | --quiet) QUIET=1 ;;
  esac
done

if [[ ! -x "$DBSRT_BIN" ]]; then
  echo "Error: $DBSRT_BIN not found or not executable. Run 'task cli:build' or 'task rs:build' first." >&2
  exit 1
fi

# Wait up to 5s for PostgreSQL to accept connections if container just started
for ((i = 1; i <= MAX_DB_RETRIES; i++)); do
  if (echo >"/dev/tcp/${DB_HOST}/${DB_PORT}") >/dev/null 2>&1; then
    break
  fi
  sleep "$DB_RETRY_INTERVAL_SECONDS"
done

# Initialize with clean CI seed fixture if registry is absent
SEED_FIXTURE="tests/fixtures/couplings.seed.json"
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
for media in "data/big_buck_bunny_trailer_480p.webm" "tests/data/big_buck_bunny_trailer_480p.mp4"; do
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
