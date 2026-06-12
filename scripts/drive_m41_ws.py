#!/usr/bin/env python3
"""M41 Step 3: WebSockets. Navigate to a page whose inline script opens a
WebSocket to the loopback echo endpoint (ws://veil/ws), sends 'hello veil' in
onopen, and receives the echo in onmessage. Proves the full RFC 6455 path:
HTTP Upgrade handshake (Sec-WebSocket-Accept), masked client frames, unmasked
server frames, send + receive. Browser needs a NIC (loopback HTTP server)."""
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 52


def type_str(d, s):
    smap = {"/": "slash", ".": "dot"}
    for ch in s:
        qcode = smap.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # address bar
    type_str(d, "/wstest.htm")
    press(d, "ret")
    check("ws page rendered", d.wait_serial("BROWSER: rendered /wstest.htm", 25, m))

    # The full WebSocket protocol path.
    check("browser opened the WebSocket", d.wait_serial("BROWSER: WebSocket open ws://veil/ws", 10, m))
    check("Upgrade handshake verified", d.wait_serial("WS: Upgrade handshake complete", 10, m))
    check("kernel server upgraded + echoed", d.wait_serial("HTTP: WebSocket upgrade on /ws", 10, m))

    # onmessage received the echo (surfaced via console.log -> browser js line).
    serial = d.serial()
    check("onmessage received the echo",
          "WS_MSG:hello veil rs=1" in serial,
          next((l for l in serial.splitlines() if "WS_MSG" in l), "no WS_MSG line"))

    d.move(950, 650)
    d.dump("m41_ws")
    d.quit()
    finish()


main()
