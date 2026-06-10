#!/usr/bin/env bash
# M24 proof: boot headless with a virtio-sound device (null audio backend
# so it runs in CI but still paces playback at the real PCM rate), play the
# on-disk 3-second test tone, and require a clean AUDIO_OK on serial.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh >/dev/null || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
S=/tmp/veil-m24-serial.log
rm -f "$S"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -drive if=none,file=disk.img,format=raw,id=hd \
    -device virtio-blk-device,drive=hd \
    -audiodev none,id=snd0 \
    -device virtio-sound-device,audiodev=snd0 \
    -fw_cfg name=opt/veil.mode,string=audio \
    -display none -serial "file:$S" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
Q=$!

ok=1
for _ in $(seq 1 300); do
    grep -q "AUDIO_OK" "$S" 2>/dev/null && { ok=0; break; }
    grep -qE "AUDIO: .* not |SND_SKIP|unsupported|KERNEL PANIC" "$S" 2>/dev/null && { ok=2; break; }
    kill -0 "$Q" 2>/dev/null || break
    sleep 0.1
done
kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null

echo "--- audio serial ---"
grep -aE "SND_OK|SND_SKIP|AUDIO|virtio-sound|stream" "$S" | tr -d '\r' || true
echo "--------------------"
if [ "$ok" -ne 0 ]; then
    echo "FAIL: no clean AUDIO_OK (ok=$ok)"
    exit 1
fi
echo "PASS: M24 — virtio-sound streamed the 3s test tone clean (AUDIO_OK)"
