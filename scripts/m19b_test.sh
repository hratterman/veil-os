#!/usr/bin/env bash
# M19b proof: boot with slirp user-networking (which NATs to the host's
# real internet) and verify the kernel resolves pool.ntp.org over its own
# DNS, exchanges a real NTP packet, and sets its wall clock to UTC. The
# timestamp on serial must land within a sane window of the host's clock,
# proving it's a real sync and not a hard-coded constant.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh >/dev/null || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
S=/tmp/veil-m19b-serial.log
rm -f "$S"

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -drive if=none,file=disk.img,format=raw,id=hd \
    -device virtio-blk-device,drive=hd \
    -netdev user,id=net0 \
    -device virtio-net-device,netdev=net0 \
    -fw_cfg name=opt/veil.mode,string=net \
    -display none -serial "file:$S" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
Q=$!

ok=1
for _ in $(seq 1 200); do
    grep -q "M19b_OK" "$S" 2>/dev/null && { ok=0; break; }
    grep -q "NTP: no sync" "$S" 2>/dev/null && { ok=2; break; }
    kill -0 "$Q" 2>/dev/null || break
    sleep 0.1
done
kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null

echo "--- relevant serial ---"
grep -E "TZ:|DNS:|NTP:|M19b_OK" "$S" || true
echo "-----------------------"

if [ "$ok" -ne 0 ]; then
    echo "FAIL: NTP sync did not complete (ok=$ok)"
    exit 1
fi

# Pull the unix timestamp the kernel set and compare to the host clock.
GUEST=$(grep "NTP: set clock to" "$S" | tail -1 | tr -d '\r' | awk '{print $NF}')
HOST=$(date +%s)
DIFF=$(( GUEST > HOST ? GUEST - HOST : HOST - GUEST ))
echo "guest NTP unix=$GUEST  host unix=$HOST  |diff|=${DIFF}s"
if [ "$DIFF" -gt 120 ]; then
    echo "FAIL: guest clock off from host by ${DIFF}s (>120s)"
    exit 1
fi

echo "PASS: M19b — real NTP sync, clock within ${DIFF}s of host"
