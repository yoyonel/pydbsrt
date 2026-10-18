#!/usr/bin/env bash
set -euo pipefail

DISPLAY_NUM=98
ASSETS_DIR="docs/assets/vlsub-dbsrt"
mkdir -p "$ASSETS_DIR"

echo "=== 1. Starting isolated Xvfb display :$DISPLAY_NUM (1920x1080x24) ==="
Xvfb ":$DISPLAY_NUM" -screen 0 1920x1080x24 -ac +extension GLX +render -noreset > /dev/null 2>&1 &
XVFB_PID=$!
sleep 1

echo "=== 2. Starting Marco Window Manager ==="
marco --display=":$DISPLAY_NUM" > /dev/null 2>&1 &
MARCO_PID=$!
sleep 0.5

echo "=== 3. Starting HD 1080p ffmpeg screen recording ==="
VIDEO_OUT="$ASSETS_DIR/vlsub_dbsrt_workflow_1080p.mp4"
ffmpeg -y -f x11grab -draw_mouse 1 -video_size 1920x1080 -i ":$DISPLAY_NUM.0" \
  -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p \
  "$VIDEO_OUT" > /dev/null 2>&1 &
FFMPEG_PID=$!
sleep 0.5

cleanup() {
  echo "=== Stopping background demo processes ==="
  kill "$VLC_PID" 2>/dev/null || true
  # Send SIGINT to ffmpeg so it writes MP4 moov atom cleanly
  kill -INT "$FFMPEG_PID" 2>/dev/null || true
  sleep 1.5
  kill "$MARCO_PID" 2>/dev/null || true
  kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT

echo "=== 4. Launching VLC Media Player ==="
export DISPLAY=":$DISPLAY_NUM"
MEDIA_FILE="${1:-tests/data/big_buck_bunny_trailer_480p.mp4}"
START_TIME="${START_TIME:-18}"
vlc --no-audio --no-sub-autodetect-file --repeat --start-time="$START_TIME" --sub-language=fr "$MEDIA_FILE" > /tmp/vlc_demo.log 2>&1 &
VLC_PID=$!

echo "Waiting for VLC player to start..."
sleep 2.5

echo "Opening VLSub (dbsrt Enhanced) dialog via View menu..."
xdotool key alt+i
sleep 0.4
xdotool key Up
sleep 0.1
xdotool key Up
sleep 0.1
xdotool key Return
sleep 2.0

echo "Capturing Step 1: Initialized Dialog..."
ffmpeg -y -f x11grab -video_size 1920x1080 -i ":$DISPLAY_NUM.0" -vframes 1 "$ASSETS_DIR/01_vlsub_dialog_init.png" > /dev/null 2>&1

VLSUB_WIN=$(xdotool search --name "VLSub" | tail -n 1)
echo "VLSub window ID: $VLSUB_WIN"

echo "Clicking 'Search by pHash'..."
xdotool mousemove --window "$VLSUB_WIN" 530 22 click 1
sleep 2.0

echo "Capturing Step 2: Search Results in Mainlist..."
ffmpeg -y -f x11grab -video_size 1920x1080 -i ":$DISPLAY_NUM.0" -vframes 1 "$ASSETS_DIR/02_vlsub_search_results.png" > /dev/null 2>&1

echo "Selecting first subtitle candidate in listbox..."
xdotool mousemove --window "$VLSUB_WIN" 200 110 click 1
sleep 0.6

echo "Clicking 'Download selection'..."
xdotool mousemove --window "$VLSUB_WIN" 395 326 click 1
sleep 2.0

echo "Capturing Step 3: Subtitle Synced and Loaded in Player..."
ffmpeg -y -f x11grab -video_size 1920x1080 -i ":$DISPLAY_NUM.0" -vframes 1 "$ASSETS_DIR/03_vlsub_subtitle_synced.png" > /dev/null 2>&1

echo "Closing dialog to observe active subtitles and OSD..."
xdotool mousemove --window "$VLSUB_WIN" 530 326 click 1
sleep 3.5

echo "=== 5. Recording successfully completed! ==="
