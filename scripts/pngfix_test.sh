#!/usr/bin/env bash
# Regression for the large-PNG OOM crash. Stages two oversized PNGs on the disk:
#   AAA2048.PNG (2048x2048) — at the decoder cap; opens first; must DECODE,
#                downscaled on the fly to fit the heap, and render (not crash,
#                not "cannot decode").
#   ZZHUGE.PNG  (3000x2000) — over the cap; must show the graceful "too large"
#                message, OS still alive.
# Before the fix either took the whole OS down (OOM panic -> semihosting exit).
# See scripts/drive_pngfix.py.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
python3 scripts/mkbigpng.py "$TMP/AAA2048.PNG" 2048 2048 || exit 2
python3 scripts/mkbigpng.py "$TMP/ZZHUGE.PNG" 3000 2000 || exit 2

scripts/mkdisk.sh --extra-dir "$TMP" >/dev/null || exit 2
exec scripts/run_gui.sh scripts/drive_pngfix.py VIEWER_OK
