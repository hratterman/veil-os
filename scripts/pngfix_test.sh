#!/usr/bin/env bash
# Regression for the large-PNG OOM crash: stage a 1920x1080 PNG on the disk
# (named to sort first so the viewer opens onto it), boot, and drive the
# viewer. The decoder must refuse it gracefully (PNG_CRASH_FIXED) and the OS
# must stay alive. See scripts/drive_pngfix.py.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
python3 scripts/mkbigpng.py "$TMP/AAABIG.PNG" 1920 1080 || exit 2

scripts/mkdisk.sh --extra-dir "$TMP" >/dev/null || exit 2
exec scripts/run_gui.sh scripts/drive_pngfix.py VIEWER_OK
