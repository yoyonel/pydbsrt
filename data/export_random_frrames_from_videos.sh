#!/bin/bash
# [Randomly extract video frames from multiple files](https://stackoverflow.com/a/25194352

INPUT_MEDIAS=$(find /tmp/pydbsrt/. -type f -exec file -N -i -- {} + | sed -n 's!: video/[^:]*$!!p')

for i in $INPUT_MEDIAS; do
  # [Linux bash: Multiple variable assignment}(https://stackoverflow.com/a/1952480)
  # https://manpages.ubuntu.com/manpages/bionic/man1/mediainfo.1.html
  read -r TOTAL_FRAMES FPS <<<"$(mediainfo --Inform="Video;%FrameCount% %FrameRate%" "$i")"

  for j in {1..3}; do
    # https://tldp.org/LDP/abs/html/randomvar.html
    RANDOM_FRAME=$((RANDOM % TOTAL_FRAMES))

    # [In BASH convert a string with . in float](https://stackoverflow.com/a/34286545)
    TIME=$(bc -l <<<"${RANDOM_FRAME}/${FPS}")
    # https://www.cyberciti.biz/faq/ksh-csh-shell-assign-store-printf-result-variable/
    TIME=$(printf "%01.3f\n" "$TIME")

    # https://www.perturb.org/display/1362_Making_ffmpeg_truly_quiet_non_verbose.html
    ffmpeg -hide_banner -loglevel error -y \
      -ss "$TIME" \
      -i "$i" \
      -frames:v 1 \
      "${i}_${RANDOM_FRAME}_${j}.jpg" 2>&1
    echo "✅ exported image: ${i}_${RANDOM_FRAME}_${j}.jpg"
  done

done
