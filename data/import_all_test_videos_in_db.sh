#!/bin/bash

# download all (open source) medias file from json file
# [public test videos](https://gist.github.com/jsturgis/3b19447b304616f18657)
# https://stedolan.github.io/jq/tutorial/
# https://stackoverflow.com/questions/1078524/how-to-specify-the-download-location-with-wget
jq -r '.categories[].videos[].sources[0]' <public_test_videos.json | xargs -n1 wget -nc -P /tmp/pydbsrt

pushd ..

# [How to search for video files on Ubuntu?](https://unix.stackexchange.com/a/106543)
find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + |
  sed -n 's!: video/[^:]*$!!p' |
  xargs -n1 poetry run pydbsrt export-imghash-from-media --media

# https://www.cyberciti.biz/faq/bash-find-exclude-all-permission-denied-messages/
ls -1 /tmp/*.phash 2>/dev/null |
  xargs -n1 poetry run pydbsrt import-images-hashes-into-db --binary-img-hash-file

popd
