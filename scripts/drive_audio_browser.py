#!/usr/bin/env python3
"""Task 1 proof: browser audio actually plays.

Loads the REAL scripts/novnc_audio.js in headless Chrome against a tiny local
HTTP+WebSocket server that streams known 16-bit stereo 44100 Hz sine PCM at
/session/test/audio (exactly the manager's framing). The page enables audio
(window.__veilAudio.enable, the ♪-click path) and, after PCM has flowed, POSTs
the Web Audio state back. We require:
  - the AudioContext reached state "running"  (autoplay unlock works)
  - >10 KB of PCM was decoded + scheduled      (the playback path runs)
  - buffers are scheduled ahead of currentTime (the lookahead is correct)

Headless Chrome runs the Web Audio render graph even with no output device, so
this validates the whole software playback path short of an actual speaker.
Emits AUDIO_BROWSER_OK on success.

No external Python or Node deps — raw-socket WS framing, stdlib HTTP server.
"""
import json
import math
import os
import socket
import struct
import subprocess
import sys
import threading
import time
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.abspath(__file__))
PORT = 7785
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

# 0.5 s of 440 Hz stereo sine, 16-bit LE, 44100 Hz — streamed in ~20ms chunks.
def make_pcm():
    sr, secs, freq, amp = 44100, 1.0, 440.0, 12000
    out = bytearray()
    for n in range(int(sr * secs)):
        v = int(amp * math.sin(2 * math.pi * freq * n / sr))
        out += struct.pack("<hh", v, v)
    return bytes(out)

PCM = make_pcm()

def ws_accept(key):
    return base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()

def ws_frame(payload):  # server->client binary, unmasked
    n = len(payload)
    if n < 126:
        hdr = struct.pack("!BB", 0x82, n)
    elif n < 65536:
        hdr = struct.pack("!BBH", 0x82, 126, n)
    else:
        hdr = struct.pack("!BBQ", 0x82, 127, n)
    return hdr + payload

RESULT = {}
RESULT_EVT = threading.Event()

PAGE = """<!doctype html><html><head><meta charset=utf-8></head><body>
<script>window.VEIL_SESSION="test";</script>
<script src="/novnc_audio.js"></script>
<script>
// Drive the real client: enable audio (the gesture path), let PCM flow, report.
function report(extra){
  var a = window.__veilAudio || {};
  var body = JSON.stringify({state:a.state, bytesPlayed:a.bytesPlayed,
    scheduledAhead:a.scheduledAhead, on:a.on, note:extra||""});
  fetch("/result",{method:"POST",body:body});
}
window.addEventListener("error", function(e){ report("jserror:"+e.message); });
window.addEventListener("load", function(){
  // Wait for the WS to connect, then enable (autoplay-policy flag lets the
  // context resume without a real gesture in headless).
  setTimeout(function(){
    try { window.__veilAudio.enable(); } catch(e){ report("enable-threw:"+e); return; }
    setTimeout(report, 1500);   // give PCM ~1.5s to stream + schedule
  }, 400);
});
</script></body></html>"""

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):  # quiet
        pass

    def do_GET(self):
        if self.path == "/" or self.path.startswith("/test"):
            return self._html(PAGE)
        if self.path == "/novnc_audio.js":
            return self._file(os.path.join(ROOT, "novnc_audio.js"), "application/javascript")
        if self.path.startswith("/session/") and self.path.endswith("/audio"):
            return self._audio_ws()
        self.send_response(404); self.end_headers()

    def do_POST(self):
        if self.path == "/result":
            n = int(self.headers.get("Content-Length", "0") or 0)
            try:
                RESULT.update(json.loads(self.rfile.read(n) or b"{}"))
            except Exception as e:
                RESULT["parse_error"] = str(e)
            RESULT_EVT.set()
            self.send_response(200); self.send_header("Content-Length","2"); self.end_headers()
            self.wfile.write(b"ok")
            return
        self.send_response(404); self.end_headers()

    def _html(self, s):
        b = s.encode()
        self.send_response(200); self.send_header("Content-Type","text/html")
        self.send_header("Content-Length", str(len(b))); self.end_headers()
        self.wfile.write(b)

    def _file(self, path, ctype):
        b = open(path, "rb").read()
        self.send_response(200); self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(b))); self.end_headers()
        self.wfile.write(b)

    def _audio_ws(self):
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            self.send_response(400); self.end_headers(); return
        self.connection.sendall((
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {ws_accept(key)}\r\n\r\n").encode())
        self.close_connection = True
        sock = self.connection
        # Stream PCM in ~20ms chunks (1764 frames * 4 bytes) for ~2.5s total.
        chunk = 1764 * 4
        try:
            for _ in range(3):                    # loop the 1s buffer ~3x
                for off in range(0, len(PCM), chunk):
                    sock.sendall(ws_frame(PCM[off:off + chunk]))
                    time.sleep(0.02)
        except OSError:
            pass

def main():
    chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    if not os.path.exists(chrome):
        print("FAIL: Google Chrome not found"); sys.exit(1)
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    profile = "/tmp/veil-chrome-audio-test"
    os.system(f"rm -rf {profile}")
    proc = subprocess.Popen([
        chrome, "--headless=new", "--disable-gpu", "--no-first-run",
        "--autoplay-policy=no-user-gesture-required",
        f"--user-data-dir={profile}",
        f"http://127.0.0.1:{PORT}/test",
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ok = RESULT_EVT.wait(timeout=30)
    proc.terminate()
    srv.shutdown()
    print("page report:", json.dumps(RESULT))
    if not ok:
        print("FAIL: page never reported (Chrome/Web Audio didn't run)"); sys.exit(1)
    state = RESULT.get("state")
    played = RESULT.get("bytesPlayed", 0) or 0
    ahead = RESULT.get("scheduledAhead", 0) or 0
    fails = []
    if state != "running":
        fails.append(f"AudioContext state={state!r} (want 'running')")
    if played <= 10240:
        fails.append(f"bytesPlayed={played} (want >10240)")
    if ahead <= 0:
        fails.append(f"scheduledAhead={ahead} (want >0)")
    if fails:
        print("FAIL:", "; ".join(fails)); sys.exit(1)
    print(f"ok   AudioContext running, {played} bytes scheduled, "
          f"{ahead:.3f}s lookahead")
    print("AUDIO_BROWSER_OK")

if __name__ == "__main__":
    main()
