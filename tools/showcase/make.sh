#!/bin/bash
# Builds the showcase video into WORKDIR. Run record_ui.sh WORKDIR first for the app footage.
# Usage: make.sh WORKDIR [OUTPUT.mp4]
set -e
W=$(realpath "$1"); OUT=${2:-$W/crtsim-showcase-vertical.mp4}; HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
CARGO_TARGET_DIR="$ROOT/target" cargo build --release --manifest-path "$HERE/shotgen/Cargo.toml"
mkdir -p "$W/shots" "$W/renders"
python3 "$HERE/gen_sources.py" "$W"
python3 "$HERE/mkshots.py" "$W"
for n in $(cat "$W/shots/order.txt"); do
  [ -f "$W/renders/$n.done" ] && continue
  "$ROOT/target/release/shotgen" "$W/shots/$n.json"
  touch "$W/renders/$n.done"
done
python3 "$HERE/music.py" "$W/music.wav"
python3 "$HERE/compose.py" "$W" "$OUT"
