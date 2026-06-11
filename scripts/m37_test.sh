#!/usr/bin/env bash
# M37 proof: the from-scratch H.264 baseline decoder and MP3 Layer III decoder,
# wired into the video player and audio app + file manager.
#
#   1. Boot self-tests (H264_OK, MP3_OK) — decoders verified against the
#      embedded test clips at boot.
#   2. GUI: open QUAD.MP4 from the file manager -> H.264 frames play.
#   3. GUI + virtio-sound: open TONE.MP3 -> the MP3 decoder feeds PCM into the
#      virtio-sound path (AUDIO_OK).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh || exit 2
scripts/mkdisk.sh >/dev/null || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil

fail() { echo "FAIL: $1"; exit 1; }

# --- 1. Boot self-tests -------------------------------------------------------
S=/tmp/veil-m37-boot.log; rm -f "$S"
qemu-system-aarch64 -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false -device ramfb \
    -device virtio-keyboard-device -device virtio-tablet-device \
    -drive if=none,file=disk.img,format=raw,id=hd0 -device virtio-blk-device,drive=hd0 \
    -display none -serial "file:$S" -no-reboot -semihosting -kernel "$KERNEL" &
Q=$!
for _ in $(seq 1 400); do
    grep -q "H264_OK" "$S" 2>/dev/null && grep -q "MP3_OK" "$S" 2>/dev/null && break
    kill -0 "$Q" 2>/dev/null || break
    sleep 0.2
done
kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null
grep -q "H264_OK" "$S" || fail "no H264_OK boot self-test"
grep -q "MP3_OK" "$S" || fail "no MP3_OK boot self-test"
echo "PASS: boot self-tests (H264_OK + MP3_OK)"

# --- 2. GUI: H.264 .mp4 in the video player -----------------------------------
scripts/run_gui.sh scripts/drive_m37_video.py WM_OK || fail "H.264 video player driver"
echo "PASS: QUAD.MP4 decodes + plays in the video player"

# --- 3a. GUI: MP3 opens in the audio app from the file manager ----------------
scripts/run_gui.sh scripts/drive_m37_mp3_open.py WM_OK || fail "MP3 file-manager dispatch"
echo "PASS: TONE.MP3 opens in the audio app + triggers playback"

# --- 3b. Headless + virtio-sound: MP3 decodes and streams to the device -------
# (The desktop kernel-task scheduler doesn't pace audio without a device; the
#  headless audio path is the deterministic end-to-end playback proof, as in M24.)
SER=/tmp/veil-m37-mp3.log; rm -f "$SER"
qemu-system-aarch64 -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -drive if=none,file=disk.img,format=raw,id=hd -device virtio-blk-device,drive=hd \
    -audiodev none,id=snd0 -device virtio-sound-device,audiodev=snd0 \
    -fw_cfg name=opt/veil.mode,string=mp3 \
    -display none -serial "file:$SER" -no-reboot -semihosting -kernel "$KERNEL" &
Q=$!
for _ in $(seq 1 400); do
    grep -q "AUDIO_OK" "$SER" 2>/dev/null && break
    grep -qE "not a decodable|KERNEL PANIC" "$SER" 2>/dev/null && break
    kill -0 "$Q" 2>/dev/null || break
    sleep 0.1
done
kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null
grep -q "AUDIO_OK" "$SER" || fail "MP3 playback never reached AUDIO_OK"
grep -q "MP3: TONE.MP3 -> 44100 Hz" "$SER" || fail "MP3 decode did not run"
echo "PASS: TONE.MP3 decoded + streamed through virtio-sound (AUDIO_OK)"

echo "ALL M37 CHECKS PASSED"
