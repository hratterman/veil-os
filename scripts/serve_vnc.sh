#!/usr/bin/env bash
# Veil OS — hosted-demo backend. Boots the full graphical OS headless with
# a VNC server on :10 (TCP 5910; :0/5900 is taken by macOS Screen
# Sharing). noVNC/websockify proxies that to a
# browser; Cloudflare fronts it at https://veil.henryratterman.com.
#
# A wrapper (launchd KeepAlive) restarts this if QEMU exits, and a 30-min
# timer (reset_vnc.sh) kills it so KeepAlive relaunches a clean desktop.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

# Rebuild the disk fresh each launch so every demo session starts clean.
scripts/mkdisk.sh >/dev/null
KERNEL=target/aarch64-unknown-none/debug/veil

echo "Veil OS is running (VNC :10 / tcp 5910)"

exec qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -drive if=none,file=disk.img,format=raw,id=hd0 \
    -device virtio-blk-device,drive=hd0 \
    -netdev user,id=net0 \
    -device virtio-net-device,netdev=net0 \
    -vnc 127.0.0.1:10 \
    -no-reboot -semihosting \
    -kernel "$KERNEL"
