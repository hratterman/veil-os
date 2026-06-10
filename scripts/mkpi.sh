#!/usr/bin/env bash
# M17: build kernel8.img + config.txt — the files to copy onto a FAT32
# SD card boot partition alongside the stock Raspberry Pi firmware
# (start4.elf, fixup4.dat, bcm2711-rpi-4-b.dtb from the firmware repo).
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

OBJCOPY="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"

# User binaries first: the kernel embeds hello.bin & friends (M9).
(cd user && cargo build --release --quiet)
for bin in hello ls cat echo spin evil; do
    "$OBJCOPY" -O binary \
        "user/target/aarch64-unknown-none/release/$bin" \
        "user/target/aarch64-unknown-none/release/$bin.bin"
done

cargo build --release --quiet --features pi4

mkdir -p pi
"$OBJCOPY" -O binary target/aarch64-unknown-none/release/veil pi/kernel8.img

cat > pi/config.txt <<'EOF'
# Veil OS on the Raspberry Pi 4.
arm_64bit=1
kernel=kernel8.img
# PL011 on GPIO 14/15 (the Bluetooth modem normally owns it).
dtoverlay=disable-bt
enable_uart=1
init_uart_clock=48000000
# Plain framebuffer scanout, no firmware overscan surprises.
disable_overscan=1
EOF

cat > pi/README.txt <<'EOF'
Veil OS — Raspberry Pi 4 boot files (M17)

1. Format an SD card with a FAT32 first partition.
2. Copy the stock Pi 4 firmware onto it from
   https://github.com/raspberrypi/firmware/tree/master/boot :
     start4.elf  fixup4.dat  bcm2711-rpi-4-b.dtb
3. Copy kernel8.img and config.txt from this directory next to them.
4. HDMI monitor in the port nearest USB-C; optional serial console at
   115200 8N1 on GPIO 14 (TXD) / 15 (RXD) / GND.

The kernel prints BOOT_OK..M17_OK on serial and composites the Veil
desktop (shell / windows / paint) to the monitor.
EOF

ls -l pi/
