#!/usr/bin/env bash
# M25 proof: two ISOLATED Veil instances, each booted from its own disk
# image with a distinct USER.TXT (alpha_fox / beta_owl). Same crossed
# -netdev dgram bridge as M20 (via scripts/hub.py) so the two real network
# stacks exchange chat broadcasts. drive_m25.py injects a message into A
# and pixel-verifies it shows up in B prefixed with A's on-disk username.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

KERNEL=target/aarch64-unknown-none/debug/veil
HUB=23556
PA=23100
PB=23101
mkdir -p shots

# Two fresh disks, each with its own baked username (mkdisk rebuilds the
# kernel incrementally on the first call; the second is disk-only fast).
scripts/mkdisk.sh --username alpha_fox --out /tmp/veil-m25-a.img >/dev/null || exit 2
scripts/mkdisk.sh --username beta_owl  --out /tmp/veil-m25-b.img >/dev/null || exit 2

dgram() {
    echo "dgram,id=net0,local.type=inet,local.host=127.0.0.1,local.port=$1,remote.type=inet,remote.host=127.0.0.1,remote.port=$HUB"
}

for p in "$HUB" "$PA" "$PB"; do
    lsof -nP -iUDP:"$p" -t 2>/dev/null | xargs -r kill 2>/dev/null
done
sleep 0.5

python3 scripts/hub.py "$HUB" &
HUBPID=$!
trap 'kill "$HUBPID" 2>/dev/null' EXIT

launch() { # name local-port ip-last-octet disk
    local name="$1" port="$2" ip="$3" disk="$4"
    rm -f "/tmp/veil-m25-$name-qmp.sock" "/tmp/veil-m25-$name-serial.log"
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb \
        -device virtio-keyboard-device \
        -device virtio-tablet-device \
        -drive "if=none,file=$disk,format=raw,id=hd" \
        -device virtio-blk-device,drive=hd \
        -netdev "$(dgram "$port")" \
        -device "virtio-net-device,netdev=net0,mac=52:54:00:12:34:0$ip" \
        -fw_cfg "name=opt/veil.net,string=10.0.0.$ip/24,,10.0.0.$ip" \
        -display none \
        -serial "file:/tmp/veil-m25-$name-serial.log" \
        -qmp "unix:/tmp/veil-m25-$name-qmp.sock,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" >"/tmp/veil-m25-$name-qemu.log" 2>&1 &
    echo $!
}

QA=$(launch a "$PA" 1 /tmp/veil-m25-a.img)
QB=$(launch b "$PB" 2 /tmp/veil-m25-b.img)

python3 scripts/drive_m25.py \
    /tmp/veil-m25-a-qmp.sock /tmp/veil-m25-a-serial.log \
    /tmp/veil-m25-b-qmp.sock /tmp/veil-m25-b-serial.log \
    "$PWD/shots"
RESULT=$?

echo "--- instance A serial (chat lines) ----------------------------"
grep -aE 'CHAT' /tmp/veil-m25-a-serial.log
echo "--- instance B serial (chat lines) ----------------------------"
grep -aE 'CHAT' /tmp/veil-m25-b-serial.log
echo "---------------------------------------------------------------"

kill "$QA" "$QB" 2>/dev/null
wait "$QA" "$QB" 2>/dev/null
[ $RESULT -eq 0 ] && echo "M25 ALL GREEN"
exit $RESULT
