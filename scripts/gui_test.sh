#!/usr/bin/env bash
# M6/M7/M8 proof run: boot with ramfb + virtio keyboard/tablet, wait for the
# desktop, then drive it through QMP input injection with pixel + serial
# assertions (scripts/drive_gui.py).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

"$(dirname "$0")/build.sh" || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
QMP_SOCK=/tmp/veil-gui-qmp.sock
SERIAL=/tmp/veil-gui-serial.log
mkdir -p shots
rm -f "$QMP_SOCK" "$SERIAL"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -display none \
    -serial "file:$SERIAL" \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
QPID=$!

for _ in $(seq 1 200); do
    grep -q 'M8_OK' "$SERIAL" 2>/dev/null && break
    kill -0 "$QPID" 2>/dev/null || break
    sleep 0.1
done

echo "--- boot serial -----------------------------------------------"
cat "$SERIAL" 2>/dev/null
echo "---------------------------------------------------------------"

if ! grep -q 'M8_OK' "$SERIAL" 2>/dev/null; then
    echo "FAIL: desktop never came up"
    kill "$QPID" 2>/dev/null
    exit 1
fi

python3 scripts/drive_gui.py "$QMP_SOCK" "$SERIAL" "$PWD/shots"
RESULT=$?
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
exit $RESULT
