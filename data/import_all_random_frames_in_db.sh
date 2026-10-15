#!/bin/bash

pushd ..

# [How to search for video files on Ubuntu?](https://unix.stackexchange.com/a/106543)
find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + |
  sed -n 's!: image/[^:]*$!!p' |
  xargs -n1 poetry run pydbsrt export-imghash-from-media --media

# https://www.cyberciti.biz/faq/bash-find-exclude-all-permission-denied-messages/
# [Use -print0/-0 or find -exec + to allow for non-alphanumeric filenames.](https://github.com/koalaman/shellcheck/wiki/SC2038)
find /tmp -name '*.phash' 2>/dev/null |
  xargs -n1 poetry run pydbsrt import-images-hashes-into-db --binary-img-hash-file

popd
