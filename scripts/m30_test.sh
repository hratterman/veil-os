#!/usr/bin/env bash
# M30 proof: pre-boot file upload. Start the session manager, POST a PNG to
# /upload, POST /boot (which bakes the upload onto a fresh disk and spawns
# QEMU), then drive the booted instance: complete first-boot setup, open
# Files, and pixel-verify the uploaded filename appears + opens in Viewer.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")/.."

SID=test123
PORT=6091
QMP="/tmp/veil-session-$SID-qmp.sock"
SERIAL="/tmp/veil-session-$SID-serial.log"
mkdir -p shots

scripts/build.sh || exit 2

# Clean any prior session state.
rm -f "$QMP" "$SERIAL" "/tmp/veil-session-$SID.img" "/tmp/veil-audio-$SID.fifo"
rm -rf "/tmp/veil-uploads-$SID"
for p in "$PORT" 6100; do lsof -nP -iTCP:"$p" -t 2>/dev/null | xargs -r kill 2>/dev/null; done

# Audio bridge taps the session FIFO so the spawned QEMU doesn't block.
node scripts/audio_server.js 6092 >/tmp/veil-m30-audio.log 2>&1 &
AUDIO=$!
python3 -u scripts/session_manager.py >/tmp/veil-m30-mgr.log 2>&1 &
MGR=$!
trap 'kill "$MGR" "$AUDIO" 2>/dev/null; pkill -f "veil-session-'"$SID"'" 2>/dev/null' EXIT
for _ in $(seq 1 50); do
    curl -s -o /dev/null "http://127.0.0.1:$PORT/healthz" && break
    sleep 0.1
done

echo "--- upload ---"
curl -s -F "file=@assets/photos/dog.png;filename=veiltest.png" \
    "http://127.0.0.1:$PORT/upload?session=$SID"
echo
echo "--- boot ---"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:$PORT/boot?session=$SID")
echo "boot HTTP $CODE"
[ "$CODE" = "302" ] || { echo "FAIL: boot did not redirect"; exit 1; }

# Wait for the spawned instance's QMP + serial.
for _ in $(seq 1 100); do [ -S "$QMP" ] && break; sleep 0.1; done
[ -S "$QMP" ] || { echo "FAIL: session QEMU never came up"; cat /tmp/veil-m30-mgr.log; exit 1; }

python3 scripts/drive_m30.py "$QMP" "$SERIAL" "$PWD/shots"
RESULT=$?

echo "--- session FILES serial ---"; grep -aE 'FILES|VIEWER: showing|SETUP' "$SERIAL" 2>/dev/null | head -30

curl -s "http://127.0.0.1:$PORT/close?session=$SID" >/dev/null 2>&1
kill "$MGR" "$AUDIO" 2>/dev/null
if [ "$RESULT" -eq 0 ]; then
    echo "UPLOAD_OK"
    echo "M30 ALL GREEN"
fi
exit $RESULT
