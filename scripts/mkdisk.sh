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
# M36: a demo source file to show the editor's syntax highlighting.
printf '// demo.rs - Veil OS\nfn main() {\n    let msg = "hello, world";\n    for i in 0..10 {\n        print(i, msg); // loop\n    }\n}\n' > "$MNT/DEMO.RS"
# M19b: local timezone, integer UTC offset in hours (EDT = -4 in summer).
printf -- '-4\n' > "$MNT/TZ.TXT"
# M41 step 21: a C program to compile + run inside Veil with `cc hello.c`.
cat > "$MNT/HELLO.C" <<'VEILC'
// Hello, Veil — compiled inside Veil by the on-OS C compiler, run in Veil.
int square(int n) { return n * n; }
int main() {
    print("Hello, Veil!");
    int sum = 0;
    for (int i = 1; i <= 5; i = i + 1) {
        sum = sum + square(i);
    }
    print_int(sum);
    return 0;
}
VEILC
# M41 step 9: a non-trivial shell script (iterate files, pipe-transform, write
# output) for the real-shell proof. Run with `sh test.sh`.
cat > "$MNT/TEST.SH" <<'VEILSH'
# Build three files, then iterate them, sort the merged contents, and write out.
echo apple > a.txt
echo banana > b.txt
echo cherry > c.txt
count=0
for f in a.txt b.txt c.txt; do
  count=$((count + 1))
  cat $f
done | sort -r > out.txt
echo "files=$count"
echo "sorted:"
cat out.txt
n=$(wc -l < out.txt)
echo "lines=$n"
if [ $n -eq 3 ]; then echo RESULT_OK; else echo RESULT_BAD; fi
VEILSH
for bin in hello ls cat echo spin evil; do
    cp "user/target/aarch64-unknown-none/release/$bin.bin" \
       "$MNT/$(echo "$bin" | tr a-z A-Z).BIN"
done
# The M15 website, served by the kernel's HTTP server.
python3 scripts/mksite.py
for f in site/*; do
    cp "$f" "$MNT/$(basename "$f" | tr a-z A-Z)"
done
# A long text file for the editor's mouse-wheel scroll test (M40 step 4).
awk 'BEGIN { for (i = 1; i <= 250; i++) printf "line %03d: the quick brown fox jumps\n", i }' \
    > "$MNT/BIG.TXT"
# Real photos for the M23 image viewer (PNG + M35 JPEG, baseline + progressive)
for f in assets/photos/*.png assets/photos/*.jpg; do
    [ -e "$f" ] || continue
    cp "$f" "$MNT/$(basename "$f" | tr a-z A-Z)"
done
cp assets/dog_baseline.jpg "$MNT/DOGBASE.JPG" 2>/dev/null || true
# M35: a demo MJPEG video (a sequence of baseline JPEG frames).
cp assets/demo.mjpeg "$MNT/DEMO.MJP" 2>/dev/null || true
# M35: WASM demo modules (hello prints via fd_write; compute is a JIT kernel).
python3 scripts/mkwasm.py assets >/dev/null 2>&1 || true
cp assets/hello.wasm "$MNT/HELLO.WSM" 2>/dev/null || true
cp assets/compute.wasm "$MNT/COMPUTE.WSM" 2>/dev/null || true
cp assets/netget.wasm "$MNT/NETGET.WSM" 2>/dev/null || true
# M41 step 12: the SDK "Hello, Veil" example app (graphical: render + on_click).
cp assets/helloapp.wasm "$MNT/HELLOAPP.WSM" 2>/dev/null || true
# M41 step 15: a malicious app that tries to read out-of-sandbox (kernel) memory.
cp assets/evil.wasm "$MNT/EVIL.WSM" 2>/dev/null || true
# M41 step 16: a non-system app that tries a network call (capability-gated).
cp assets/nettry.wasm "$MNT/NETTRY.WSM" 2>/dev/null || true
# M24 audio: a 3-second 440 Hz sine test tone (16-bit stereo 44.1 kHz).
python3 scripts/mkwav.py "$MNT/TONE.WAV" 3 >/dev/null
# M37 codecs: a from-scratch-decoded MP3 (Layer III) and H.264 baseline MP4.
cp assets/codec/tone.mp3 "$MNT/TONE.MP3" 2>/dev/null || true
cp assets/codec/quad.mp4 "$MNT/QUAD.MP4" 2>/dev/null || true
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
    for f in "$SRC"/*.png "$SRC"/*.PNG "$SRC"/*.jpg "$SRC"/*.JPG "$SRC"/*.jpeg "$SRC"/*.wav "$SRC"/*.WAV "$SRC"/*.gif "$SRC"/*.GIF; do
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
