#!/usr/bin/env python3
"""Browser-audio proof: drive a session to play the tone while a WebSocket
client (mimicking novnc_audio.js) reads /session/<sid>/audio from the
manager, and require it to receive a substantial amount of PCM.

Usage: drive_audio_ws.py <qmp> <serial> <shots> <sid> <manager_port>"""
import base64
import os
import socket
import sys
import threading

from guilib import Driver, check, finish


def key(d, qc):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qc}}}])


def ws_read_pcm(sid, port, out, stop):
    """Connect to /session/<sid>/audio, read WS binary frames, total bytes."""
    s = socket.create_connection(("127.0.0.1", port), timeout=10)
    k = base64.b64encode(os.urandom(16)).decode()
    s.sendall((
        f"GET /session/{sid}/audio HTTP/1.1\r\nHost: x\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {k}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    ).encode())
    buf = s.recv(4096)  # the 101 handshake
    out["handshake"] = b"101" in buf
    s.settimeout(6)
    data = b""
    try:
        while not stop.is_set():
            data += s.recv(65536)
            if not data:
                break
            # Parse server->client binary frames (unmasked), tally payloads.
            i = 0
            while i + 2 <= len(data):
                b1, b2 = data[i], data[i + 1]
                ln = b2 & 0x7F
                hdr = 2
                if ln == 126:
                    if i + 4 > len(data):
                        break
                    ln = int.from_bytes(data[i + 2:i + 4], "big"); hdr = 4
                elif ln == 127:
                    if i + 10 > len(data):
                        break
                    ln = int.from_bytes(data[i + 2:i + 10], "big"); hdr = 10
                if i + hdr + ln > len(data):
                    break
                if b1 & 0x0F == 0x2:  # binary frame
                    out["pcm"] += ln
                i += hdr + ln
            data = data[i:]
    except OSError:
        pass
    finally:
        s.close()


def main():
    qmp, serial, shots, sid, port = sys.argv[1:6]
    port = int(port)
    d = Driver(qmp, serial, shots)

    check("setup screen shown", d.wait_serial("SETUP: first boot", 60))
    d.type_text("demo")
    key(d, "ret")
    check("desktop reached", d.wait_serial("WM_OK", 25))

    mark = len(d.serial())
    d.click(70 + 7 * 78 + 36, 768 - 20)  # Audio launcher (idx 7 with NIC)
    check("audio window open", d.wait_serial("AUDIO: window open", 5, mark))

    out = {"pcm": 0, "handshake": False}
    stop = threading.Event()
    t = threading.Thread(target=ws_read_pcm, args=(sid, port, out, stop), daemon=True)
    t.start()

    mark = len(d.serial())
    d.click(512, 427)  # Play
    check("stream started", d.wait_serial("SND: stream started", 5, mark))
    check("stream completed (AUDIO_OK, no freeze)", d.wait_serial("AUDIO_OK", 25, mark))
    stop.set()
    t.join(timeout=8)

    check("audio WS handshake (101)", out["handshake"])
    check(f"browser received PCM over WS ({out['pcm']} bytes)", out["pcm"] > 100_000)
    d.quit()
    finish()


if __name__ == "__main__":
    main()
