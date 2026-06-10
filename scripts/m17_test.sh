#!/usr/bin/env bash
# M17 proof run (emulated half): boot the exact SD-card artifact
# (pi/kernel8.img) on QEMU's raspi4b machine — BCM2711 peripherals, the
# real mailbox property interface — and require the boot sentinels plus
# a screendump showing the composited desktop. The other half of the
# pass criterion (a physical Pi 4 + HDMI monitor) is a human step;
# pi/README.txt has the SD card instructions.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkpi.sh || exit 2
SERIAL=/tmp/veil-m17-serial.log
QMP_SOCK=/tmp/veil-m17-qmp.sock
mkdir -p shots
rm -f "$SERIAL" "$QMP_SOCK"

qemu-system-aarch64 \
    -machine raspi4b \
    -display none \
    -serial "file:$SERIAL" \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    -no-reboot -semihosting \
    -kernel pi/kernel8.img &
QPID=$!

FAILED=0
result() {
    if [ "$2" -eq 0 ]; then echo "PASS: $1"; else echo "FAIL: $1"; FAILED=1; fi
}

for _ in $(seq 1 300); do
    grep -q 'M17_OK' "$SERIAL" 2>/dev/null && break
    kill -0 "$QPID" 2>/dev/null || break
    sleep 0.1
done

echo "--- raspi4b serial --------------------------------------------"
cat "$SERIAL" 2>/dev/null
echo "---------------------------------------------------------------"

for s in "BOOT_OK: veil kernel alive on BCM2711" "PI: ARM RAM" "PI_FB: 1024x768 32bpp" \
         "MMU_ON" "HEAP_OK" "PI_FB_OK" "M17_OK"; do
    grep -q "$s" "$SERIAL" 2>/dev/null; result "serial '$s'" $?
done

python3 - "$QMP_SOCK" "$PWD/shots" <<'EOF'
import sys
sys.path.insert(0, "scripts")
from guilib import Qmp, Image, check, check_px, failures
import time

q = Qmp(sys.argv[1])
shots = sys.argv[2]
time.sleep(0.3)
q.cmd("screendump", filename=f"{shots}/m17_pi_desktop.ppm", format="ppm")
q.cmd("screendump", filename=f"{shots}/m17_pi_desktop.png", format="png")
time.sleep(0.2)
img = Image(f"{shots}/m17_pi_desktop.ppm")
check("framebuffer is 1024x768", (img.w, img.h) == (1024, 768), f"{img.w}x{img.h}")
# Desktop background (0xff28_4858) and the focused (topmost: paint)
# window's title bar (0xff30_60c0), straight from wm.rs.
check_px(img, "desktop background", 1000, 10, (0x28, 0x48, 0x58))
check_px(img, "focused title bar (paint)", 600, 340, (0x30, 0x60, 0xC0))
# Paint canvas starts white.
check_px(img, "paint canvas", 700, 500, (0xFF, 0xFF, 0xFF))
q.cmd("quit")
sys.exit(1 if failures else 0)
EOF
result "screendump pixel checks" $?

kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
if [ "$FAILED" -eq 0 ]; then echo "M17 (emulated) ALL GREEN"; fi
exit $FAILED
