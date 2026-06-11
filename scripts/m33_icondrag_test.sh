#!/usr/bin/env bash
# M33 Task 4 proof: desktop icon drag + reboot persistence. Boot 1 drags the
# 'edit' icon onto the 'clock' slot (DRAG_OK) and writes ICONS.TXT. Boot 2
# boots the SAME disk and must log the reordered icon order from ICONS.TXT.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/build.sh >/tmp/veil-m33i-build.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/veil-m33i-build.log; exit 2; }
scripts/mkdisk.sh >/dev/null 2>&1 || exit 2   # fresh disk: no ICONS.TXT yet

KERNEL=target/aarch64-unknown-none/debug/veil
QMP=/tmp/veil-m33i-qmp.sock
SERIAL=/tmp/veil-m33i-serial.log
boot() {  # $1 = serial log
    rm -f "$QMP" "$1"
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb -device virtio-keyboard-device -device virtio-tablet-device \
        -drive "if=none,file=disk.img,format=raw,id=hd" -device virtio-blk-device,drive=hd \
        -display none -serial "file:$1" -qmp "unix:$QMP,server,nowait" \
        -no-reboot -semihosting -kernel "$KERNEL" >/tmp/veil-m33i-qemu.log 2>&1 &
    echo $!
}

# --- Boot 1: perform the drag ---
S1=/tmp/veil-m33i-serial1.log
Q=$(boot "$S1")
for _ in $(seq 1 200); do [ -S "$QMP" ] && grep -q 'WM_OK' "$S1" 2>/dev/null && break; sleep 0.1; done
python3 scripts/drive_m33_icondrag.py "$QMP" "$S1" "$PWD/shots"
R1=$?
kill "$Q" 2>/dev/null; wait "$Q" 2>/dev/null

# --- Boot 2: same disk, confirm the order was restored ---
S2=/tmp/veil-m33i-serial2.log
Q=$(boot "$S2")
trap 'kill "$Q" 2>/dev/null' EXIT
for _ in $(seq 1 200); do grep -q 'WM_OK' "$S2" 2>/dev/null && break; sleep 0.1; done
ORDER=$(grep -aE 'ICONS: order =' "$S2" | tail -1)
kill "$Q" 2>/dev/null

echo "boot2 $ORDER"
R2=0
echo "$ORDER" | grep -q 'order = clock edit browser' || { echo "FAIL: order not persisted (got: $ORDER)"; R2=1; }
[ "$R1" -eq 0 ] && [ "$R2" -eq 0 ] && echo "ICONDRAG ALL GREEN"
[ "$R1" -eq 0 ] && [ "$R2" -eq 0 ]
