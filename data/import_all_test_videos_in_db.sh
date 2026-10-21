#!/bin/bash

# download all (open source) medias file from json file
# [public test videos](https://gist.github.com/jsturgis/3b19447b304616f18657)
# https://stedolan.github.io/jq/tutorial/
# https://stackoverflow.com/questions/1078524/how-to-specify-the-download-location-with-wget
jq -r '.categories[].videos[].sources[0]' <public_test_videos.json | xargs -n1 wget -nc -P /tmp/pydbsrt

pushd .. || exit 1

# [How to search for video files on Ubuntu?](https://unix.stackexchange.com/a/106543)
find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + |
  sed -n 's!: video/[^:]*$!!p' |
  xargs -n1 uv run pydbsrt export-imghash-from-media --media

# https://www.cyberciti.biz/faq/bash-find-exclude-all-permission-denied-messages/
find /tmp -maxdepth 1 -name '*.phash' -print0 2>/dev/null |
  xargs -0 -n1 uv run pydbsrt import-images-hashes-into-db --binary-img-hash-file

popd || exit 1
