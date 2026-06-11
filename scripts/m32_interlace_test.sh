#!/usr/bin/env bash
# M32-C proof: Adam7 interlaced PNG. Generate a plain + an Adam7-interlaced
# copy of the same gradient, build a disk carrying both, boot (no NIC — the
# Viewer is taskbar idx 5 without a NIC, which the driver assumes), and let
# drive_m32_interlace.py cycle the Viewer to each and compare them pixel-for-
# pixel. The decoder emits INTERLACE_OK when it decodes the interlaced one.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh >/tmp/veil-m32i-build.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/veil-m32i-build.log; exit 2; }
GRADDIR=/tmp/veil-grad
rm -rf "$GRADDIR"; mkdir -p "$GRADDIR"
python3 scripts/mkgrad.py "$GRADDIR" || exit 2
scripts/mkdisk.sh --extra-dir "$GRADDIR" >/dev/null 2>&1 || exit 2

KERNEL=target/aarch64-unknown-none/debug/veil
QMP=/tmp/veil-m32i-qmp.sock
SERIAL=/tmp/veil-m32i-serial.log
mkdir -p shots
rm -f "$QMP" "$SERIAL"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -drive "if=none,file=disk.img,format=raw,id=hd" \
    -device virtio-blk-device,drive=hd \
    -display none -serial "file:$SERIAL" \
    -qmp "unix:$QMP,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" >/tmp/veil-m32i-qemu.log 2>&1 &
Q=$!
trap 'kill "$Q" 2>/dev/null' EXIT
for _ in $(seq 1 200); do [ -S "$QMP" ] && grep -q 'WM_OK' "$SERIAL" 2>/dev/null && break; sleep 0.1; done

python3 scripts/drive_m32_interlace.py "$QMP" "$SERIAL" "$PWD/shots"
RESULT=$?
echo "--- INTERLACE serial ---"; grep -aE 'INTERLACE_OK|VIEWER: showing GRAD' "$SERIAL" | head
exit $RESULT
