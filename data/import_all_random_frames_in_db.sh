#!/bin/bash

pushd .. || exit 1

# [How to search for video files on Ubuntu?](https://unix.stackexchange.com/a/106543)
find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + |
  sed -n 's!: image/[^:]*$!!p' |
  xargs -n1 uv run pydbsrt export-imghash-from-media --media

# https://www.cyberciti.biz/faq/bash-find-exclude-all-permission-denied-messages/
# [Use -print0/-0 or find -exec + to allow for non-alphanumeric filenames.](https://github.com/koalaman/shellcheck/wiki/SC2038)
find /tmp -name '*.phash' -print0 2>/dev/null |
  xargs -0 -n1 uv run pydbsrt import-images-hashes-into-db --binary-img-hash-file

popd || exit 1
