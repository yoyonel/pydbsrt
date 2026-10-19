#!/usr/bin/env bash
# Auto-discovers and certifies ground-truth (video/.phash <-> .srt) couplings
# into data/SRT/couplings.json and PostgreSQL (if reachable).
set -euo pipefail

DBSRT_BIN="${DBSRT_BIN:-./target/release/dbsrt}"
PHASH_DIR="${PHASH_DIR:-data/__PHASH__}"
SUBTITLES_DIR="${SUBTITLES_DIR:-data/SRT}"
TMP_PHASH_DIR="${TMP_PHASH_DIR:-/tmp}"
DB_HOST="${PSQL_IP:-127.0.0.1}"
DB_PORT="${PSQL_PORT:-5433}"

if [[ ! -x "$DBSRT_BIN" ]]; then
  echo "Error: $DBSRT_BIN not found or not executable. Run 'task rs:build' first." >&2
  exit 1
fi

# Wait up to 5s for PostgreSQL to accept connections if container just started
for _ in {1..10}; do
  if (echo >"/dev/tcp/${DB_HOST}/${DB_PORT}") >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

certify_media_pair() {
  local media_path="$1"
  local srt_path="$2"
  local lang="$3"
  "$DBSRT_BIN" certify-coupling \
    --media "$media_path" \
    --subtitle "$srt_path" \
    --lang "$lang" \
    --phash-dir "$TMP_PHASH_DIR" \
    --subtitles-dir "$SUBTITLES_DIR" \
    --db-host "$DB_HOST" \
    --db-port "$DB_PORT"
}

certify_phash_pair() {
  local phash_path="$1"
  local srt_path="$2"
  local lang="$3"
  "$DBSRT_BIN" certify-coupling \
    --phash-file "$phash_path" \
    --subtitle "$srt_path" \
    --lang "$lang" \
    --subtitles-dir "$SUBTITLES_DIR" \
    --db-host "$DB_HOST" \
    --db-port "$DB_PORT" \
    --allow-oob
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
