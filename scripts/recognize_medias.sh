#!/usr/bin/env bash

PROJECT_ROOT_DIR=$(git rev-parse --show-toplevel)
NAS_ROOT_DIR="${NAS_ROOT_DIR:-$HOME/__SYNOLOGY__}"
MEDIAS_ROOT_DIR="${MEDIAS_ROOTDIR:-$NAS_ROOT_DIR/tvshow/Silicon Valley/}"
PARALLEL_NB_JOBS="${PARALLEL_NB_JOBS:-1}"

find "${MEDIAS_ROOT_DIR}" -type f -name "*.mkv" | \
/usr/bin/time -v \
  parallel -j"${PARALLEL_NB_JOBS}" --eta -I% --max-args 1 --tty \
    poetry run python "${PROJECT_ROOT_DIR}"/src/pydbsrt/app_cli.py recognize-media -m % --output_format CSV