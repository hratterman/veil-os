#!/usr/bin/env bash
# M16 proof run: full system (graphics + input + disk + net). The desktop
# opens the browser on the OS's own homepage; scripts/drive_m16.py then
# pixel-checks the render and click-navigates the site through QMP.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
QMP_SOCK=/tmp/veil-m16-qmp.sock
SERIAL=/tmp/veil-m16-serial.log
mkdir -p shots
rm -f "$QMP_SOCK" "$SERIAL"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -drive if=none,file=disk.img,format=raw,id=hd \
    -device virtio-blk-device,drive=hd \
    -netdev user,id=net0 \
    -device virtio-net-device,netdev=net0 \
    -display none \
    -serial "file:$SERIAL" \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
QPID=$!

for _ in $(seq 1 400); do
    grep -q 'M8_OK' "$SERIAL" 2>/dev/null && break
    kill -0 "$QPID" 2>/dev/null || break
    sleep 0.1
done

if ! grep -q 'M8_OK' "$SERIAL" 2>/dev/null; then
    echo "FAIL: desktop never came up"
    cat "$SERIAL" 2>/dev/null
    kill "$QPID" 2>/dev/null
    exit 1
fi

python3 scripts/drive_m16.py "$QMP_SOCK" "$SERIAL" "$PWD/shots"
RESULT=$?

echo "--- serial (browser lines) ------------------------------------"
grep -E 'BROWSER|M16|SRV: GET' "$SERIAL"
echo "---------------------------------------------------------------"

kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
exit $RESULT
