#!/usr/bin/env bash
# M25 deploy cutover: replace the single-instance hosted demo
# (com.veil.qemu + com.veil.websockify + com.veil.reset) with the
# per-visitor session manager (com.veil.sessions).
#
# This is an OUTWARD-FACING change to the live demo box. It is intentionally
# a separate, manually-run script (not part of any test) so the cutover is
# deliberate. The Cloudflare tunnel ingress must also route the /session/
# prefix + bare / to 127.0.0.1:6090 — the manager binds dual-stack on 6090
# (the port the live os.henryratterman.com route already targets; cloudflared
# dials localhost as IPv6 [::1], so the dual-stack bind is required).
set -eu
LA="$HOME/Library/LaunchAgents"
SRC="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/Library/Logs/veil"

echo "Unloading legacy single-instance agents..."
for a in com.veil.qemu com.veil.websockify com.veil.reset; do
    launchctl unload "$LA/$a.plist" 2>/dev/null || true
    rm -f "$LA/$a.plist"
done

echo "Installing com.veil.relay + com.veil.audio + com.veil.sessions..."
for a in com.veil.relay com.veil.audio com.veil.sessions; do
    cp "$SRC/launchd/$a.plist" "$LA/$a.plist"
    launchctl unload "$LA/$a.plist" 2>/dev/null || true
    launchctl load "$LA/$a.plist"
done

echo "Done. Relay on :7778, session manager on localhost:6090 (dual-stack)."
echo "Remember: point the Cloudflare tunnel ( / and /session/ ) at 127.0.0.1:6090."
