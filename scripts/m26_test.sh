#!/usr/bin/env bash
# M26 proof: three Veil instances connected to one TCP chat relay
# (scripts/relay.py). Each instance uses its own slirp `user` netdev and
# reaches the host relay at the slirp gateway 10.0.2.2:7778 (fw_cfg
# opt/veil.relay). drive_m26.py drives a public broadcast + a DM and
# pixel-verifies routing (DM reaches bob, not cid) and the online roster.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

KERNEL=target/aarch64-unknown-none/debug/veil
RELAY_PORT=7778
mkdir -p shots

# Build one kernel + three disks, each with a distinct baked username.
scripts/mkdisk.sh --username ann --out /tmp/veil-m26-ann.img >/dev/null || exit 2
scripts/mkdisk.sh --username bob --out /tmp/veil-m26-bob.img >/dev/null || exit 2
scripts/mkdisk.sh --username cid --out /tmp/veil-m26-cid.img >/dev/null || exit 2

lsof -nP -iTCP:"$RELAY_PORT" -t 2>/dev/null | xargs -r kill 2>/dev/null
sleep 0.3
python3 scripts/relay.py "$RELAY_PORT" >/tmp/veil-m26-relay.log 2>&1 &
RELAYPID=$!
trap 'kill "$RELAYPID" 2>/dev/null' EXIT

launch() { # name disk
    local name="$1" disk="$2"
    rm -f "/tmp/veil-m26-$name-qmp.sock" "/tmp/veil-m26-$name-serial.log"
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb \
        -device virtio-keyboard-device \
        -device virtio-tablet-device \
        -drive "if=none,file=$disk,format=raw,id=hd" \
        -device virtio-blk-device,drive=hd \
        -netdev user,id=net0 \
        -device virtio-net-device,netdev=net0 \
        -fw_cfg "name=opt/veil.relay,string=10.0.2.2:$RELAY_PORT" \
        -display none \
        -serial "file:/tmp/veil-m26-$name-serial.log" \
        -qmp "unix:/tmp/veil-m26-$name-qmp.sock,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" >"/tmp/veil-m26-$name-qemu.log" 2>&1 &
    echo $!
}

QA=$(launch ann /tmp/veil-m26-ann.img)
QB=$(launch bob /tmp/veil-m26-bob.img)
QC=$(launch cid /tmp/veil-m26-cid.img)

python3 scripts/drive_m26.py \
    /tmp/veil-m26-ann-qmp.sock /tmp/veil-m26-ann-serial.log \
    /tmp/veil-m26-bob-qmp.sock /tmp/veil-m26-bob-serial.log \
    /tmp/veil-m26-cid-qmp.sock /tmp/veil-m26-cid-serial.log \
    "$PWD/shots"
RESULT=$?

echo "--- relay log ---"; cat /tmp/veil-m26-relay.log
for n in ann bob cid; do
    echo "--- $n chat serial ---"
    grep -aE 'CHAT|DM_OK' "/tmp/veil-m26-$n-serial.log"
done

kill "$QA" "$QB" "$QC" 2>/dev/null
wait "$QA" "$QB" "$QC" 2>/dev/null
[ $RESULT -eq 0 ] && echo "M26 ALL GREEN"
exit $RESULT
