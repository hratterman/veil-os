#!/usr/bin/env bash
# M23 proof: build the disk (which preloads sample PNGs), then drive the
# image viewer — open it, verify CHECK.PNG renders (checker colors), Right
# arrow changes the image, Left returns. See scripts/drive_m23.py.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh >/dev/null || exit 2
exec scripts/run_gui.sh scripts/drive_m23.py VIEWER_OK
