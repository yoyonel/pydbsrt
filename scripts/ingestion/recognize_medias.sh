#!/usr/bin/env bash

PROJECT_ROOT_DIR=$(git rev-parse --show-toplevel)
MEDIA_ROOT_DIR="${MEDIA_ROOT_DIR:-$HOME/media}"
MEDIAS_ROOT_DIR="${MEDIAS_ROOTDIR:-$MEDIA_ROOT_DIR}"
PARALLEL_NB_JOBS="${PARALLEL_NB_JOBS:-1}"

find "${MEDIAS_ROOT_DIR}" -type f -name "*.mkv" |
  /usr/bin/time -v \
    parallel -j"${PARALLEL_NB_JOBS}" --eta -I% --max-args 1 --tty \
    uv run python "${PROJECT_ROOT_DIR}"/src/pydbsrt/app_cli.py recognize-media -m % --output_format CSV
