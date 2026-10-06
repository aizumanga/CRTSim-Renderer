#!/usr/bin/env bash
# Run the game headless under Xvfb + Mesa and dump, from inside
# ValkyrieGame::ConfigCallback_RebuildNTSCLUT, the palette MakePalette generates and the
# 1024x32 NTSC LUT copied into NTSC_LUT_Texture. Addresses are for SuperGame_NFML
# sha256 cb4145a9...2f0723 (itch.io Linux build, "September 25, 2020 Update", engine 4348,
# game 1039). Work on a copy of the game directory.
#
# usage: capture_ntsc_lut.sh GAME_DIR OUT_DIR
# needs: gdb, Xvfb, i386 libc/libstdc++/libgl1 + libgl1-mesa-dri:i386
set -euo pipefail
game=$(realpath "$1"); out=$(realpath "$2"); mkdir -p "$out/home"
cat > "$out/lut.gdb" <<GDB
set pagination off
set confirm off
handle SIGPIPE nostop noprint
# MakePalette(pal, tint, I, Q) about to be called: log its arguments
break *0x0813dd5c
commands
  silent
  printf "MakePalette tint=%f I=%f Q=%f\n", *(float*)(\$esp+4), *(float*)(\$esp+8), *(float*)(\$esp+12)
  continue
end
# MakePalette returned: ebx = NColor8[256]
break *0x0813dd61
commands
  silent
  dump binary memory $out/palette.bin \$ebx \$ebx+1024
  continue
end
# NTexture::lock returned: eax = texture memory; NImage width/height on the stack
break *0x0813dd97
commands
  silent
  set \$base = \$eax
  set \$w = *(int*)(\$esp+0x464)
  set \$h = *(int*)(\$esp+0x468)
  printf "LUT %dx%d\n", \$w, \$h
  continue
end
# copy loop finished, before unlock
break *0x0813dde8
commands
  silent
  dump binary memory $out/ntsc_lut.bin \$base \$base+\$w*\$h*4
  kill
  quit
end
run
GDB
Xvfb :99 -screen 0 1280x960x24 >/dev/null 2>&1 & xvfb=$!
trap 'kill $xvfb' EXIT
sleep 1
cd "$game"
DISPLAY=:99 SDL_AUDIODRIVER=dummy LD_LIBRARY_PATH=. HOME="$out/home" \
  gdb -q -batch -x "$out/lut.gdb" ./SuperGame_NFML 2>&1 | grep -E 'MakePalette|LUT'
ls -l "$out/palette.bin" "$out/ntsc_lut.bin"
