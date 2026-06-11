#!/usr/bin/env bash
# M33 Lisp proofs (persistence / file I/O). Builds a FRESH disk (so LISP.TXT
# and any test files start absent), boots no-NIC (lisp is taskbar idx 9), and
# runs the given driver. Usage: m33_lisp_test.sh <driver.py>
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

DRIVER="${1:?usage: m33_lisp_test.sh <driver.py>}"
scripts/build.sh >/tmp/veil-m33-build.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/veil-m33-build.log; exit 2; }
scripts/mkdisk.sh >/dev/null 2>&1 || exit 2   # fresh disk: no LISP.TXT yet

KERNEL=target/aarch64-unknown-none/debug/veil
QMP=/tmp/veil-m33-qmp.sock
SERIAL=/tmp/veil-m33-serial.log
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
    -kernel "$KERNEL" >/tmp/veil-m33-qemu.log 2>&1 &
Q=$!
trap 'kill "$Q" 2>/dev/null' EXIT
for _ in $(seq 1 200); do [ -S "$QMP" ] && grep -q 'WM_OK' "$SERIAL" 2>/dev/null && break; sleep 0.1; done

python3 "$DRIVER" "$QMP" "$SERIAL" "$PWD/shots"
RESULT=$?
echo "--- LISP serial ---"; grep -aE 'LISP_EVAL|LISP: restored|LISP_PERSIST_OK|LISP_IO_OK|WM: closed' "$SERIAL" | tail -20
exit $RESULT
