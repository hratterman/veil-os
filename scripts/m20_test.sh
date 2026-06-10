#!/usr/bin/env bash
# M20 proof: two complete Veil instances on one Mac, bridged by QEMU's
# -netdev socket (A listens, B connects), exchanging chat messages over
# UDP broadcast — both kernels, both network stacks, ours on both ends.
# scripts/drive_m20.py injects typing into A and pixel-verifies the
# message in B (and the reverse), using the kernel's own font table.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
BRIDGE=127.0.0.1:23456
mkdir -p shots

launch() { # name extra-netdev-arg ip
    local name="$1" netarg="$2" ip="$3"
    rm -f "/tmp/veil-m20-$name-qmp.sock" "/tmp/veil-m20-$name-serial.log"
    # qemu's own stdout/stderr must NOT inherit the $() capture pipe, or
    # the command substitution blocks until qemu exits.
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb \
        -device virtio-keyboard-device \
        -device virtio-tablet-device \
        -netdev "socket,id=net0,$netarg" \
        -device "virtio-net-device,netdev=net0,mac=52:54:00:12:34:0$3" \
        -fw_cfg "name=opt/veil.net,string=10.0.0.$3/24,,10.0.0.$3" \
        -display none \
        -serial "file:/tmp/veil-m20-$name-serial.log" \
        -qmp "unix:/tmp/veil-m20-$name-qmp.sock,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" >"/tmp/veil-m20-$name-qemu.log" 2>&1 &
    echo $!
}

QA=$(launch a "listen=$BRIDGE" 1)
sleep 1
QB=$(launch b "connect=$BRIDGE" 2)

python3 scripts/drive_m20.py \
    /tmp/veil-m20-a-qmp.sock /tmp/veil-m20-a-serial.log \
    /tmp/veil-m20-b-qmp.sock /tmp/veil-m20-b-serial.log \
    "$PWD/shots"
RESULT=$?

echo "--- instance A serial (chat lines) ----------------------------"
grep -aE 'CHAT' /tmp/veil-m20-a-serial.log
echo "--- instance B serial (chat lines) ----------------------------"
grep -aE 'CHAT' /tmp/veil-m20-b-serial.log
echo "---------------------------------------------------------------"

kill "$QA" "$QB" 2>/dev/null
wait "$QA" "$QB" 2>/dev/null
[ $RESULT -eq 0 ] && echo "M20 ALL GREEN"
exit $RESULT
