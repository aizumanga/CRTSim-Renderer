#!/usr/bin/env bash
# Starts Super Win the Game's Windows build under Wine (Direct3D 9, which Wine draws through
# OpenGL) on its own Xvfb display, drives it from the title into the first scene of a new game
# with the keys play_to_gameplay.sh sends, and grabs 40 frames of the screen into OUT.
#
# usage: play_windows.sh GAME_DIR OUT_DIR [WINEPREFIX]
#   GAME_DIR  the installed game: SuperGame.exe beside its .npk and .vpk files (7z x the
#             installer)
# needs: wine32 (Wine 9), Xvfb, xdotool, ffmpeg
# The game runs in a Wine virtual desktop: without a window manager it otherwise never becomes
# the active window, and pauses. Wine needs a sound device, so ALSA gets a silent one. The
# game's settings are in WINEPREFIX/drive_c/users/$USER/Documents/My Games/Super Win the Game.
set -euo pipefail
game=$(realpath "$1"); out=$(realpath -m "$2")
export WINEPREFIX=$(realpath -m "${3:-$out/wineprefix}") WINEDEBUG=-all DISPLAY=:97
mkdir -p "$out/home"
printf 'pcm.!default { type null }\nctl.!default { type hw card 0 }\n' > "$out/home/.asoundrc"
Xvfb :97 -screen 0 1280x960x24 >/dev/null 2>&1 &
echo $! > "$out/xvfb.pid"
sleep 1
[ -d "$WINEPREFIX" ] || wineboot -i >/dev/null 2>&1
exe=$(winepath -w "$game/SuperGame.exe")
(cd "$game" && HOME=$out/home wine explorer /desktop=swtg,1280x960 "$exe" > "$out/game.log" 2>&1 &)
# A new prefix takes a while to set up before the game's window appears.
for _ in $(seq 120); do xdotool search --name "Super Win" >/dev/null 2>&1 && break; sleep 1; done
sleep 18                                   # the Minor Key Games logo
xdotool mousemove 640 480 click 1
key() { xdotool keydown "$1"; sleep 0.15; xdotool keyup "$1"; sleep "${2:-2}"; }
key Return 3; key Return 4; key Return 4
# The opening dialogue, until its box is gone: its top border is a bright line across the
# screen near the top, where the scene behind it is dark.
dialogue() {
  import -window root -crop 1040x14+120+78 +repage txt:- 2>/dev/null | python3 -c '
import sys, re
v = [sum(map(int, m.groups())) / 3 for m in re.finditer(r": \((\d+),(\d+),(\d+)", sys.stdin.read())]
sys.exit(0 if v and sum(v) / len(v) > 40 else 1)'
}
for _ in $(seq 12); do dialogue || break; key Return 4; done
sleep 6
mkdir -p "$out/frames"
ffmpeg -loglevel error -f x11grab -framerate 30 -video_size 1280x960 -i :97 -frames:v 40 \
  "$out/frames/f%03d.png"
wineserver -k
kill "$(cat "$out/xvfb.pid")"
