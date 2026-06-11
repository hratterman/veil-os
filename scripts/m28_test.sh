#!/usr/bin/env bash
# M28 proof: browser audio via PCM-over-WebSocket. Boot headless with the
# `wav` audiodev writing to a FIFO; run the Node audio bridge that taps the
# FIFO and forwards PCM over a WebSocket; connect ws_probe.js and require it
# to read >= 4096 predominantly-non-zero PCM bytes once the Audio app plays.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

KERNEL=target/aarch64-unknown-none/debug/veil
SID=m28
FIFO="/tmp/veil-audio-$SID.fifo"
PORT=6092
QMP=/tmp/veil-m28-qmp.sock
SERIAL=/tmp/veil-m28-serial.log
mkdir -p shots

scripts/mkdisk.sh >/dev/null || exit 2
rm -f "$FIFO" "$QMP" "$SERIAL"; mkfifo "$FIFO"

# Free a stale bridge port and start the audio bridge.
lsof -nP -iTCP:"$PORT" -t 2>/dev/null | xargs -r kill 2>/dev/null
node scripts/audio_server.js "$PORT" >/tmp/veil-m28-bridge.log 2>&1 &
BRIDGE=$!
sleep 0.5

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb \
    -device virtio-keyboard-device \
    -device virtio-tablet-device \
    -drive "if=none,file=disk.img,format=raw,id=hd" \
    -device virtio-blk-device,drive=hd \
    -audiodev "wav,id=snd0,path=$FIFO" \
    -device virtio-sound-device,audiodev=snd0 \
    -display none -serial "file:$SERIAL" \
    -qmp "unix:$QMP,server,nowait" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" >/tmp/veil-m28-qemu.log 2>&1 &
Q=$!
trap 'kill "$Q" "$BRIDGE" 2>/dev/null; rm -f "$FIFO"' EXIT

for _ in $(seq 1 100); do [ -S "$QMP" ] && break; sleep 0.1; done

# Connect the WS probe BEFORE playback so it captures the whole tone.
node scripts/ws_probe.js "$PORT" "$SID" 4096 30000 >/tmp/veil-m28-probe.log 2>&1 &
PROBE=$!
sleep 0.5

python3 scripts/drive_m28.py "$QMP" "$SERIAL" "$PWD/shots"
DRV=$?

wait "$PROBE"; PR=$?

echo "--- audio serial ---"; grep -aE "SND|AUDIO" "$SERIAL" | head
echo "--- bridge log ---"; cat /tmp/veil-m28-bridge.log
echo "--- probe result ---"; cat /tmp/veil-m28-probe.log

kill "$Q" "$BRIDGE" 2>/dev/null; wait "$Q" 2>/dev/null; rm -f "$FIFO"
if [ "$DRV" -eq 0 ] && [ "$PR" -eq 0 ] && grep -q AUDIO_STREAM_OK /tmp/veil-m28-probe.log; then
    echo "M28 ALL GREEN"
    exit 0
fi
echo "M28 FAILED (driver=$DRV probe=$PR)"
exit 1
