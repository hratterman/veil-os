#!/usr/bin/env bash
# Build a FAT16 disk image (superfloppy, no partition table) holding the
# user binaries + a README, using macOS-native tools. The host-side FAT
# tooling doubles as an independent check of our FAT16 driver's output.
set -eu
cd "$(dirname "$0")/.."

IMG=disk.img
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
sync
hdiutil detach "$(echo "$MNT" | sed 's|/Volumes/.*||; s|^|/dev/null|')" >/dev/null 2>&1 || \
    hdiutil detach "$MNT" >/dev/null

echo "disk.img ready:"
ls -l "$IMG"
