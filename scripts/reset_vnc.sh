#!/usr/bin/env bash
# Kill the running hosted-demo QEMU (VNC on tcp 5910) so its launchd
# KeepAlive guardian relaunches a fresh desktop with a clean disk image.
# Wired to run every 30 minutes (com.veil.reset).
PID=$(lsof -nP -iTCP:5910 -sTCP:LISTEN -t 2>/dev/null | head -1)
if [ -n "$PID" ]; then
    echo "$(date '+%F %T') resetting veil demo (qemu pid $PID)"
    kill "$PID"
fi
