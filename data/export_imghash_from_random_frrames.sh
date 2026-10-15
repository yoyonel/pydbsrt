#!/bin/bash
# [Randomly extract video frames from multiple files](https://stackoverflow.com/a/25194352

INPUT_IMAGES=$(find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + | sed -n 's!: image/[^:]*$!!p')

pushd ..

for i in $INPUT_IMAGES; do
  poetry run pydbsrt export-imghash-from-media -r $i
done

popd