#!/usr/bin/env bash
# Boot the full system (graphics + input + disk) headless and hand control
# to a python QMP driver. Usage: run_gui.sh <driver.py> <wait-sentinel>
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

DRIVER="${1:?usage: run_gui.sh <driver.py> <sentinel>}"
SENTINEL="${2:?}"

scripts/build.sh || exit 2
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
    -drive if=none,file=disk.img,format=raw,id=hd0 \
    -device virtio-blk-device,drive=hd0 \
    -display none \
    -serial "file:$SERIAL" \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
QPID=$!

for _ in $(seq 1 200); do
    grep -q "$SENTINEL" "$SERIAL" 2>/dev/null && break
    kill -0 "$QPID" 2>/dev/null || break
    sleep 0.1
done

if ! grep -q "$SENTINEL" "$SERIAL" 2>/dev/null; then
    echo "FAIL: never saw $SENTINEL on serial"
    cat "$SERIAL" 2>/dev/null
    kill "$QPID" 2>/dev/null
    exit 1
fi

python3 "$DRIVER" "$QMP_SOCK" "$SERIAL" "$PWD/shots"
RESULT=$?
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
exit $RESULT
