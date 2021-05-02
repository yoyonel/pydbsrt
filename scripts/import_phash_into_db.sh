#!/usr/bin/env bash

PROJECT_ROOT_DIR=$(git rev-parse --show-toplevel)
PHASH_ROOT_DIR="${MEDIAS_ROOTDIR:-$HOME/__SYNOLOGY__/tvshow/__PHASH__}"
PARALLEL_NB_JOBS="${PARALLEL_NB_JOBS:-1}"

pushd "${PROJECT_ROOT_DIR}"/docker
docker-compose up -d
popd

/usr/bin/ls -1 "${PHASH_ROOT_DIR}"/*.phash | \
/usr/bin/time -v \
    parallel -j"${PARALLEL_NB_JOBS}" --eta -I% --max-args 1 --tty \
        poetry run python "${PROJECT_ROOT_DIR}"/src/pydbsrt/app_cli.py import-images-hashes-into-db -r %
