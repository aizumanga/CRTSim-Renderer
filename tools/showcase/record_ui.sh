#!/bin/bash
# Records the desktop app on a virtual X display: playing the video test card, dragging the
# Compare divider, then cycling the five interface themes. Coordinates assume the app's default
# 1280x850 window with the Sky Diary theme, the welcome already acknowledged and a fresh session.
# Usage: record_ui.sh WORKDIR   (needs Xvfb, xdotool, ffmpeg and a Vulkan driver such as lavapipe)
set -e
W=$1; ROOT=$(cd "$(dirname "$0")/../.." && pwd)
Xvfb :99 -screen 0 1600x1000x24 & XV=$!
export DISPLAY=:99
mkdir -p "$W/xdg" "$W/appdata"
XDG_RUNTIME_DIR="$W/xdg" CRTSIM_DATA_DIR="$W/appdata" "$ROOT/target/release/crtsim-desktop" > "$W/app.log" 2>&1 & APP=$!
sleep 15
xdotool mousemove 213 15 click 1; sleep 1; xdotool mousemove 247 113 click 1; sleep 8   # File > Video test card
ffmpeg -hide_banner -loglevel error -y -f x11grab -framerate 30 -video_size 1280x850 -i :99.0+0,0 \
  -c:v libx264 -preset ultrafast -crf 12 -t 30 "$W/ui_rec.mp4" & FF=$!
sleep 1; xdotool mousemove 422 705; sleep 0.4; xdotool click 1                           # Play
sleep 3.5; xdotool mousemove 547 67; sleep 0.3; xdotool click 1                          # Compare
sleep 1.5; xdotool mousemove 830 400 mousedown 1
for x in $(seq 830 -12 520) $(seq 520 14 1180) $(seq 1180 -12 830); do xdotool mousemove $x 400; sleep 0.02; done
xdotool mouseup 1
sleep 1; xdotool mousemove 484 67; sleep 0.3; xdotool click 1                            # CRT
sleep 1.5
for y in 70 136 168 102 202; do                                                          # View > each theme
  xdotool mousemove 334 15; sleep 0.3; xdotool click 1; sleep 0.6
  xdotool mousemove 346 $y; sleep 0.4; xdotool click 1; sleep 2
done
wait $FF; kill $APP $XV
mkdir -p "$W/uiframes"; ffmpeg -hide_banner -loglevel error -y -i "$W/ui_rec.mp4" "$W/uiframes/%04d.png"
