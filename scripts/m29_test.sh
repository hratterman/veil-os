#!/usr/bin/env bash
# M29 proof: in-OS file manager. Boot the desktop, open Files from the
# taskbar, verify the file list renders (highlight + first filename in exact
# font pixels), click the first PNG and verify the Viewer opens with the
# decoded image. No NIC (Files is taskbar idx 7).
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

KERNEL=target/aarch64-unknown-none/debug/veil
QMP=/tmp/veil-m29-qmp.sock
SERIAL=/tmp/veil-m29-serial.log
mkdir -p shots

scripts/mkdisk.sh >/dev/null || exit 2
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
    -kernel "$KERNEL" >/tmp/veil-m29-qemu.log 2>&1 &
Q=$!
trap 'kill "$Q" 2>/dev/null' EXIT
for _ in $(seq 1 100); do [ -S "$QMP" ] && break; sleep 0.1; done

python3 scripts/drive_m29.py "$QMP" "$SERIAL" "$PWD/shots"
RESULT=$?

echo "--- FILES serial ---"; grep -aE 'FILES|VIEWER: showing' "$SERIAL" | head -30

kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null
if [ "$RESULT" -eq 0 ]; then
    echo "FILES_OK"
    echo "M29 ALL GREEN"
fi
exit $RESULT
