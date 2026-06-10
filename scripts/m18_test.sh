#!/usr/bin/env bash
# M18 proof: type into the editor, SAV, reboot on the same disk, LOD,
# pixel-verify the text reappears (scripts/drive_m18.py, two phases).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh || exit 2   # fresh disk: NOTE.TXT must not exist yet
KERNEL=target/aarch64-unknown-none/debug/veil
QMP_SOCK=/tmp/veil-m18-qmp.sock
SERIAL=/tmp/veil-m18-serial.log
mkdir -p shots

boot_and_drive() { # phase
    rm -f "$QMP_SOCK" "$SERIAL"
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb \
        -device virtio-keyboard-device \
        -device virtio-tablet-device \
        -drive if=none,file=disk.img,format=raw,id=hd0 \
        -device virtio-blk-device,drive=hd0 \
        -display none \
        -serial "file:$SERIAL" \
        -qmp "unix:$QMP_SOCK,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" &
    local qpid=$!
    for _ in $(seq 1 200); do
        grep -q 'EDITOR: window open' "$SERIAL" 2>/dev/null && break
        kill -0 "$qpid" 2>/dev/null || break
        sleep 0.1
    done
    python3 scripts/drive_m18.py "$QMP_SOCK" "$SERIAL" "$PWD/shots" "$1"
    local rc=$?
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
    return $rc
}

echo "=== boot 1: type + SAV ========================================"
boot_and_drive save || exit 1
echo "=== boot 2: LOD + pixel-verify ================================"
boot_and_drive load || exit 1
echo "M18 ALL GREEN"
