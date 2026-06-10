#!/usr/bin/env bash
# M5 proof run: boot with ramfb (headless display), wait for the kernel's
# M5_OK on serial, screendump via QMP, verify exact pixels, leave a PNG.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

cargo build --quiet || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
QMP_SOCK=/tmp/veil-qmp.sock
SERIAL=/tmp/veil-serial.log
mkdir -p shots
rm -f "$QMP_SOCK" "$SERIAL" shots/m5.ppm shots/m5.png

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -device ramfb \
    -display none \
    -serial "file:$SERIAL" \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    -fw_cfg name=opt/veil.mode,string=m5scene \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
QPID=$!

for _ in $(seq 1 200); do
    grep -q 'M5_OK' "$SERIAL" 2>/dev/null && break
    kill -0 "$QPID" 2>/dev/null || break
    sleep 0.1
done

echo "--- serial output ---------------------------------------------"
cat "$SERIAL" 2>/dev/null
echo "---------------------------------------------------------------"

if ! grep -q 'M5_OK' "$SERIAL" 2>/dev/null; then
    echo "FAIL: kernel never reported M5_OK"
    kill "$QPID" 2>/dev/null
    exit 1
fi

python3 scripts/qmp.py "$QMP_SOCK" "$PWD/shots/m5.ppm" "$PWD/shots/m5.png" || {
    kill "$QPID" 2>/dev/null
    exit 1
}
wait "$QPID" 2>/dev/null

python3 scripts/verify_m5.py shots/m5.ppm || exit 1
echo "PASS: M5 screenshot verified (shots/m5.png)"
