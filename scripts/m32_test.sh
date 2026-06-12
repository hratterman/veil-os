#!/usr/bin/env bash
# M32 proof: browser overhaul (scroll/history/table/internet) + Lisp + Adam7.
# The browser drivers need a NIC (the homepage is fetched from the in-kernel
# HTTP server over the net stack) and the internet driver needs the host
# proxy (veil_proxy.py on 127.0.0.1:7779, reached by the guest at 10.0.2.2).
# Usage: m32_test.sh <driver.py> [extra qemu args...]
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

DRIVER="${1:?usage: m32_test.sh <driver.py>}"; shift || true
scripts/build.sh >/tmp/veil-m32-build.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/veil-m32-build.log; exit 2; }
scripts/mkdisk.sh >/dev/null 2>&1 || exit 2

KERNEL=target/aarch64-unknown-none/debug/veil
QMP=/tmp/veil-m32-qmp.sock
SERIAL=/tmp/veil-m32-serial.log
mkdir -p shots
rm -f "$QMP" "$SERIAL"

# Start the host proxy (idempotent — skip if 7779 is already taken).
PROXY_PID=""
if ! nc -z 127.0.0.1 7779 2>/dev/null; then
    python3 -u scripts/veil_proxy.py >/tmp/veil-proxy-test.log 2>&1 &
    PROXY_PID=$!
fi

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -smp 4 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device virtio-gpu-device \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -drive "if=none,file=disk.img,format=raw,id=hd" \
    -device virtio-blk-device,drive=hd \
    -netdev user,id=net0 \
    -device virtio-net-device,netdev=net0 \
    -display none -serial "file:$SERIAL" \
    -qmp "unix:$QMP,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" >/tmp/veil-m32-qemu.log 2>&1 &
Q=$!
cleanup() { kill "$Q" 2>/dev/null; [ -n "$PROXY_PID" ] && kill "$PROXY_PID" 2>/dev/null; }
trap cleanup EXIT
# Debug boot runs all the heavy self-tests (mp3/h264/js/es6/jit/ws) before the
# desktop comes up, so allow up to ~50s to reach WM_OK.
for _ in $(seq 1 500); do [ -S "$QMP" ] && grep -q 'WM_OK' "$SERIAL" 2>/dev/null && break; sleep 0.1; done

python3 "$DRIVER" "$QMP" "$SERIAL" "$PWD/shots"
RESULT=$?

echo "--- BROWSER serial ---"; grep -aE 'BROWSER|SCROLL_OK|HISTORY_OK|TABLE_OK|INTERNET_OK' "$SERIAL" | tail -25
exit $RESULT
