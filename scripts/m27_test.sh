#!/usr/bin/env bash
# M27 proof: first-boot setup screen. Boot a disk with no USER.TXT, drive
# the setup screen (type a name, arrow timezone to UTC-5, Enter), confirm
# the desktop appears. Reboot the SAME disk and confirm the setup screen is
# skipped and the values persisted (USER.TXT=testuser, TZ.TXT=-5).
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

KERNEL=target/aarch64-unknown-none/debug/veil
DISK=/tmp/veil-m27.img
mkdir -p shots

scripts/mkdisk.sh --no-user --out "$DISK" >/dev/null || exit 2

boot() { # name
    local name="$1"
    rm -f "/tmp/veil-m27-$name-qmp.sock" "/tmp/veil-m27-$name-serial.log"
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a72 -m 512M \
        -global virtio-mmio.force-legacy=false \
        -device ramfb \
        -device virtio-keyboard-device \
        -device virtio-tablet-device \
        -drive "if=none,file=$DISK,format=raw,id=hd" \
        -device virtio-blk-device,drive=hd \
        -netdev user,id=net0 \
        -device virtio-net-device,netdev=net0 \
        -display none \
        -serial "file:/tmp/veil-m27-$name-serial.log" \
        -qmp "unix:/tmp/veil-m27-$name-qmp.sock,server,nowait" \
        -no-reboot -semihosting \
        -kernel "$KERNEL" >"/tmp/veil-m27-$name-qemu.log" 2>&1 &
    local pid=$!
    for _ in $(seq 1 50); do
        [ -S "/tmp/veil-m27-$name-qmp.sock" ] && break
        sleep 0.1
    done
    echo "$pid"
}

# --- boot 1: setup ---------------------------------------------------------
Q1=$(boot b1)
python3 scripts/drive_m27.py setup /tmp/veil-m27-b1-qmp.sock /tmp/veil-m27-b1-serial.log "$PWD/shots"
R1=$?
kill "$Q1" 2>/dev/null; wait "$Q1" 2>/dev/null

# --- host-side file check (independent of the kernel) ----------------------
MNT=$(hdiutil attach "$DISK" 2>/dev/null | grep -o '/Volumes/.*')
USER_TXT=$(tr -d '\r\n' < "$MNT/USER.TXT" 2>/dev/null || echo MISSING)
TZ_TXT=$(tr -d '\r\n' < "$MNT/TZ.TXT" 2>/dev/null || echo MISSING)
hdiutil detach "$MNT" >/dev/null 2>&1
echo "on-disk USER.TXT='$USER_TXT'  TZ.TXT='$TZ_TXT'"
[ "$USER_TXT" = "testuser" ] && echo "ok   USER.TXT persisted" || { echo "FAIL USER.TXT"; R1=1; }
[ "$TZ_TXT" = "-5" ] && echo "ok   TZ.TXT persisted" || { echo "FAIL TZ.TXT"; R1=1; }

# --- boot 2: setup must be skipped -----------------------------------------
Q2=$(boot b2)
python3 scripts/drive_m27.py verify /tmp/veil-m27-b2-qmp.sock /tmp/veil-m27-b2-serial.log "$PWD/shots"
R2=$?
kill "$Q2" 2>/dev/null; wait "$Q2" 2>/dev/null

echo "--- boot1 SETUP serial ---"; grep -aE 'SETUP' /tmp/veil-m27-b1-serial.log
echo "--- boot2 (no SETUP expected) ---"; grep -aE 'SETUP|CHAT: window|TZ:' /tmp/veil-m27-b2-serial.log

if [ "$R1" -eq 0 ] && [ "$R2" -eq 0 ]; then
    echo "M27 ALL GREEN"
    exit 0
fi
echo "M27 FAILED (boot1=$R1 boot2=$R2)"
exit 1
