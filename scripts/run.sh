#!/usr/bin/env bash
# Veil OS — one-shot setup and run for macOS.
# Installs deps if missing, clones/updates the repo, builds, and boots.
# Usage: curl -fsSL https://raw.githubusercontent.com/hratterman/veil-os/main/scripts/run.sh | bash
set -euo pipefail

REPO="https://github.com/hratterman/veil-os.git"
DIR="$HOME/veil-os"

echo "=== Veil OS ==="

# 1. Homebrew
if ! command -v brew &>/dev/null; then
  echo "Installing Homebrew..."
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  eval "$(/opt/homebrew/bin/brew shellenv)" 2>/dev/null || eval "$(/usr/local/bin/brew shellenv)" 2>/dev/null
fi

# 2. QEMU
if ! command -v qemu-system-aarch64 &>/dev/null; then
  echo "Installing QEMU..."
  brew install qemu
fi

# 3. Rust
if ! command -v cargo &>/dev/null; then
  echo "Installing Rust..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
export PATH="$HOME/.cargo/bin:$PATH"

# 4. AArch64 bare-metal target
if ! rustup target list --installed | grep -q aarch64-unknown-none; then
  echo "Adding aarch64-unknown-none target..."
  rustup target add aarch64-unknown-none
fi

# 5. Clone or update
if [ -d "$DIR/.git" ]; then
  echo "Updating repo..."
  git -C "$DIR" pull --ff-only
else
  echo "Cloning veil-os..."
  git clone "$REPO" "$DIR"
fi

cd "$DIR"

# 6. Build user-space binaries + kernel (build.sh handles objcopy ELF -> .bin)
echo "Building (this takes ~30s the first time)..."
bash scripts/build.sh 2>&1 | tail -5

# 7. Disk image (drop your own .png/.wav into user-files/ to add them — they
#    show up in the Files app inside the OS).
if [ ! -d "$DIR/user-files" ]; then
  mkdir -p "$DIR/user-files"
  echo "Tip: drop .png or .wav files into $DIR/user-files/ and re-run to add them to the OS."
fi
scripts/mkdisk.sh --no-user >/dev/null

# 8. Start the host HTTP proxy so the in-OS browser can reach the real
#    internet (the guest connects to it at the slirp gateway 10.0.2.2:7779).
#    Skip if the port is already held by a prior run.
if ! nc -z 127.0.0.1 7779 2>/dev/null; then
  python3 -u scripts/veil_proxy.py >/tmp/veil-proxy.log 2>&1 &
  echo "Started browser internet proxy (pid $!, log /tmp/veil-proxy.log)."
fi

# 9. Boot
echo ""
echo "Booting Veil OS — close the window to quit."
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
  -kernel target/aarch64-unknown-none/debug/veil
