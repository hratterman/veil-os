#!/usr/bin/env bash
# Veil OS — local one-liner launcher (macOS).
#
# Builds the FAT16 disk, then boots the full graphical OS in a native
# Cocoa window with user-mode networking (so the clock NTP-syncs, the
# browser fetches the on-disk site, and chat can reach a relay). Close the
# window to quit.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh --no-user >/dev/null
KERNEL=target/aarch64-unknown-none/debug/veil

# Start the host HTTP proxy so the in-OS browser can reach the real internet
# (the guest connects to it at the slirp gateway 10.0.2.2:7779). Skip if the
# port is already held — e.g. the hosted-demo launchd agent or a prior run.
if ! nc -z 127.0.0.1 7779 2>/dev/null; then
    python3 -u scripts/veil_proxy.py >/tmp/veil-proxy.log 2>&1 &
    echo "Started browser internet proxy (pid $!, log /tmp/veil-proxy.log)."
fi

echo "Veil OS is running — close the window to quit."

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
    -audiodev coreaudio,id=snd0 \
    -device virtio-sound-device,audiodev=snd0 \
    -display cocoa \
    -no-reboot -semihosting \
    -kernel "$KERNEL"
