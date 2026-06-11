#!/usr/bin/env bash
# M33 Task 5 proof: a from-scratch TLS 1.3 handshake to example.com:443 over
# slirp, then an HTTP GET that must return 200 over the encrypted channel.
# Boots headless with a NIC and -fw_cfg opt/veil.tls=1 (which gates the
# boot-time TLS attempt). Needs real internet.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh >/tmp/veil-tls-build.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/veil-tls-build.log; exit 2; }
KERNEL=target/aarch64-unknown-none/debug/veil
SERIAL=/tmp/veil-tls-serial.log
rm -f "$SERIAL"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -netdev user,id=net0 -device virtio-net-device,netdev=net0 \
    -fw_cfg name=opt/veil.tls,string=1 \
    -display none -serial "file:$SERIAL" \
    -no-reboot -semihosting -kernel "$KERNEL" >/tmp/veil-tls-qemu.log 2>&1 &
Q=$!
trap 'kill "$Q" 2>/dev/null' EXIT

for _ in $(seq 1 600); do
    grep -qaE 'TLS_OK|TLS: no 200|TLS: handshake failed|KERNEL PANIC' "$SERIAL" 2>/dev/null && break
    kill -0 "$Q" 2>/dev/null || break
    sleep 0.1
done
kill "$Q" 2>/dev/null

echo "--- TLS serial ---"
grep -aE 'TLS|CRYPTO_OK|DNS:' "$SERIAL" | tail -25
grep -qa 'TLS_OK' "$SERIAL" && { echo "TLS ALL GREEN"; exit 0; }
echo "TLS FAILED"; exit 1
