#!/usr/bin/env bash
# Starts Super Win the Game headless (Xvfb + Mesa) with capture_shim.so and drives it from the
# title screen into the first scene of a new game, where nothing overlays the CRT. Then write
# OUT/go (see capture_shim.c) to feed it a probe.
#
# usage: play_to_gameplay.sh GAME_DIR SHIM_SO OUT_DIR [HOME_DIR]
# needs: Xvfb, xdotool, ImageMagick (import), i386 libc/libstdc++/libgl1 + Mesa
# Keys go through XTEST to the focused window; SDL ignores events sent to a window directly.
set -euo pipefail
game=$(realpath "$1"); shim=$(realpath "$2"); out=$(realpath -m "$3")
home=$(realpath -m "${4:-$out/home}")
mkdir -p "$out" "$home"; rm -f "$out/go"
export DISPLAY=:99
pgrep -x Xvfb >/dev/null || { Xvfb :99 -screen 0 1280x960x24 >/dev/null 2>&1 & sleep 1; }
cd "$game"
SHIM_DIR=$out SDL_AUDIODRIVER=dummy LD_LIBRARY_PATH=. LD_PRELOAD=$shim HOME=$home \
  ./SuperGame_NFML > "$out/game.log" 2>&1 &
echo $! > "$out/game.pid"
sleep 14                                   # the Minor Key Games logo
window=$(xdotool search --name "Super Win" | head -1)
xdotool windowfocus --sync "$window" 2>/dev/null || true
xdotool mousemove 640 480 click 1
key() { xdotool keydown "$1"; sleep 0.15; xdotool keyup "$1"; sleep "${2:-2}"; }
key Return 3                               # title: press Enter
key Return 4                               # Start a New Game
key Return 4                               # Start Original Game
for _ in 1 2 3 4 5 6; do key Return 2.5; done   # the opening dialogue
sleep 4
import -window root -resize 480x360 "$out/gameplay.png"
