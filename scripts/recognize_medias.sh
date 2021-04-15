#!/usr/bin/env bash

PROJECT_ROOT_DIR=$(git rev-parse --show-toplevel)
MEDIAS_ROOT_DIR="${MEDIAS_ROOTDIR:-$HOME/NAS/tvshow/Silicon Valley/}"
PARALLEL_NB_JOBS="${PARALLEL_NB_JOBS:-1}"

find "${MEDIAS_ROOT_DIR}" -type f -name "*.mkv" | \
/usr/bin/time -v \
  parallel -j"${PARALLEL_NB_JOBS}" --eta -I% --max-args 1 --tty \
    poetry run python "${PROJECT_ROOT_DIR}"/src/pydbsrt/app_cli.py recognize-media -m % --output_format CSV