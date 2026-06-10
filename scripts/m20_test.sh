#!/usr/bin/env bash
# M20 proof: two complete Veil instances on one Mac, bridged by a pair of
# crossed QEMU -netdev dgram tunnels (each instance sends UDP-encapsulated
# ethernet to the other's port and listens on its own) — symmetric and
# fully bidirectional, unlike socket listen/connect (one direction only on
# this QEMU) or mcast (no delivery on macOS). UDP broadcasts ride the
# tunnel as ordinary frames. Both kernels, both network stacks, ours on
# both ends. drive_m20.py injects typing into A and pixel-verifies the
# message in B (and the reverse), using the kernel's own font table.
# scripts/drive_m20.py injects typing into A and pixel-verifies the
# message in B (and the reverse), using the kernel's own font table.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
HUB=23456  # host-side reflector hub port (scripts/hub.py)
PA=23000   # instance A's local dgram port
PB=23001   # instance B's local dgram port
mkdir -p shots

dgram() { # local-port  -> tunnel this instance's frames to the hub
    echo "dgram,id=net0,local.type=inet,local.host=127.0.0.1,local.port=$1,remote.type=inet,remote.host=127.0.0.1,remote.port=$HUB"
}

# Free the dgram/hub ports from any prior run still shutting down, so
# back-to-back invocations don't race on a still-bound UDP port.
for p in "$HUB" "$PA" "$PB"; do
    lsof -nP -iUDP:"$p" -t 2>/dev/null | xargs -r kill 2>/dev/null
done
sleep 0.5

python3 scripts/hub.py "$HUB" &
HUBPID=$!
trap 'kill "$HUBPID" 2>/dev/null' EXIT

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
        -netdev "$netarg" \
        -device "virtio-net-device,netdev=net0,mac=52:54:00:12:34:0$3" \
        -fw_cfg "name=opt/veil.net,string=10.0.0.$3/24,,10.0.0.$3" \
        -display none \
        -serial "file:/tmp/veil-m20-$name-serial.log" \
        -qmp "unix:/tmp/veil-m20-$name-qmp.sock,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" >"/tmp/veil-m20-$name-qemu.log" 2>&1 &
    echo $!
}

QA=$(launch a "$(dgram $PA)" 1)
QB=$(launch b "$(dgram $PB)" 2)

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
