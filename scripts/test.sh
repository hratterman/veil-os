#!/usr/bin/env bash
# Veil test harness (spec §6): boot the kernel headless in QEMU, capture
# serial, grep for the milestone's sentinel, and fail on timeout or a
# non-zero semihosting exit. Usage:
#
#   scripts/test.sh <sentinel> [timeout-seconds] [extra qemu args...]
#
# Builds first, so `scripts/test.sh 'BOOT_OK'` is the whole M1 check.
set -u
export PATH="$HOME/.cargo/bin:$PATH"

SENTINEL="${1:?usage: test.sh <sentinel> [timeout] [qemu args...]}"
TIMEOUT="${2:-20}"
shift; [ $# -gt 0 ] && shift

"$(dirname "$0")/build.sh" || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil

LOG="$(mktemp)"
TIMED_OUT="$(mktemp)" && rm -f "$TIMED_OUT"
trap 'rm -f "$LOG" "$TIMED_OUT"' EXIT

qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -nographic \
    -no-reboot -no-shutdown \
    -semihosting \
    -kernel "$KERNEL" \
    "$@" >"$LOG" 2>&1 &
QPID=$!

# Watchdog: kill QEMU if the kernel hangs (macOS has no coreutils timeout).
(
    for _ in $(seq 1 "$((TIMEOUT * 10))"); do
        kill -0 "$QPID" 2>/dev/null || exit 0
        sleep 0.1
    done
    touch "$TIMED_OUT"
    kill "$QPID" 2>/dev/null
) &
WPID=$!

wait "$QPID"
QSTATUS=$?
kill "$WPID" 2>/dev/null
wait "$WPID" 2>/dev/null

echo "--- serial output ---------------------------------------------"
cat "$LOG"
echo "---------------------------------------------------------------"

if [ -e "$TIMED_OUT" ]; then
    echo "FAIL: timeout after ${TIMEOUT}s, QEMU killed by watchdog"
    exit 1
fi
if ! grep -q "$SENTINEL" "$LOG"; then
    echo "FAIL: sentinel '$SENTINEL' not found (qemu exit: $QSTATUS)"
    exit 1
fi
if [ "$QSTATUS" -ne 0 ]; then
    echo "FAIL: sentinel found but guest exit code $QSTATUS"
    exit 1
fi
echo "PASS: sentinel '$SENTINEL' found, guest exited 0"
