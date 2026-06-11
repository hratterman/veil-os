#!/usr/bin/env bash
# Build a FAT16 disk image (superfloppy, no partition table) holding the
# user binaries + a README, using macOS-native tools. The host-side FAT
# tooling doubles as an independent check of our FAT16 driver's output.
set -eu
cd "$(dirname "$0")/.."

IMG=disk.img
USERNAME=""
EXTRA_DIR=""
NO_USER=0
while [ $# -gt 0 ]; do
    case "$1" in
        --username) USERNAME="$2"; shift 2;;
        --out)      IMG="$2"; shift 2;;
        --extra-dir) EXTRA_DIR="$2"; shift 2;;
        --no-user)  NO_USER=1; shift;;
        *) echo "mkdisk: unknown arg '$1'" >&2; exit 2;;
    esac
done

scripts/build.sh

rm -f "$IMG"
dd if=/dev/zero of="$IMG" bs=1m count=16 2>/dev/null

DEV=$(hdiutil attach -nomount "$IMG" | awk 'NR==1{print $1}')
/sbin/newfs_msdos -F 16 -v VEILFS "$DEV" >/dev/null
hdiutil detach "$DEV" >/dev/null

MNT=$(hdiutil attach "$IMG" | grep -o '/Volumes/.*')
printf 'Hello from the Veil filesystem! This file was written by macOS.\n' > "$MNT/README.TXT"
# M19b: local timezone, integer UTC offset in hours (EDT = -4 in summer).
printf -- '-4\n' > "$MNT/TZ.TXT"
for bin in hello ls cat echo spin; do
    cp "user/target/aarch64-unknown-none/release/$bin.bin" \
       "$MNT/$(echo "$bin" | tr a-z A-Z).BIN"
done
# The M15 website, served by the kernel's HTTP server.
python3 scripts/mksite.py
for f in site/*; do
    cp "$f" "$MNT/$(basename "$f" | tr a-z A-Z)"
done
# Real photos for the M23 image viewer
for f in assets/photos/*.png; do
    cp "$f" "$MNT/$(basename "$f" | tr a-z A-Z)"
done
# M24 audio: a 3-second 440 Hz sine test tone (16-bit stereo 44.1 kHz).
python3 scripts/mkwav.py "$MNT/TONE.WAV" 3 >/dev/null
# M25/M27: USER.TXT labels Chat and gates the first-boot setup screen.
#   --no-user  -> omit it, so the OS shows the setup screen on boot (hosted
#                 demo, m27 test); --username NAME bakes a specific name;
#                 otherwise a default "guest" so local/regression boots go
#                 straight to the desktop.
if [ "$NO_USER" = 1 ]; then
    :
elif [ -n "$USERNAME" ]; then
    printf '%.20s\n' "$USERNAME" > "$MNT/USER.TXT"
else
    printf 'guest\n' > "$MNT/USER.TXT"
fi
# M29: user-supplied media dropped into ./user-files/ (png + wav only).
# M30: an explicit upload directory staged by the session manager.
for SRC in user-files "$EXTRA_DIR"; do
    [ -n "$SRC" ] && [ -d "$SRC" ] || continue
    for f in "$SRC"/*.png "$SRC"/*.PNG "$SRC"/*.wav "$SRC"/*.WAV; do
        [ -e "$f" ] || continue
        cp "$f" "$MNT/$(basename "$f" | tr a-z A-Z)" 2>/dev/null || \
            echo "mkdisk: skipped $(basename "$f") (disk full?)" >&2
    done
done
sync
hdiutil detach "$(echo "$MNT" | sed 's|/Volumes/.*||; s|^|/dev/null|')" >/dev/null 2>&1 || \
    hdiutil detach "$MNT" >/dev/null

echo "disk.img ready:"
ls -l "$IMG"
