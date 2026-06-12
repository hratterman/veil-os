#!/usr/bin/env python3
"""Veil OS hosted-demo session manager (M25, extended in M30).

Replaces the single static QEMU+websockify with a per-visitor instance:
every connection gets its own randomly-named, freshly-built disk image and
an isolated QEMU on a private VNC port, fronted by a per-session websockify.

  GET  /                      landing page (M30) — upload + Boot button
  POST /upload?session=<id>   stage .png/.wav uploads (M30)
  POST /boot?session=<id>     build disk, spawn QEMU+websockify, 302 -> noVNC
  GET  /session/<id>/...      reverse-proxied to that session's websockify

Self-contained: Python stdlib only. Run `session_manager.py --selftest`
to exercise username generation, port allocation and disk building without
a browser.

Layout:
  - up to MAX_SESSIONS concurrent instances, VNC :11.. , websockify 6100..
  - 30 min of inactivity (or an explicit GET /close) reclaims a session:
    QEMU killed, disk + upload dir removed, ports freed.
"""
import base64
import hashlib
import html
import json
import os
import random
import re
import select
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
AUDIO_CHUNK = 4096  # ~23 ms at 44100 Hz, 16-bit stereo


def ws_accept(key):
    return base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()


def ws_binary_frame(payload):
    """Frame `payload` as a single unmasked binary WebSocket message."""
    n = len(payload)
    if n < 126:
        head = bytes([0x82, n])
    elif n < 65536:
        head = bytes([0x82, 126]) + n.to_bytes(2, "big")
    else:
        head = bytes([0x82, 127]) + n.to_bytes(8, "big")
    return head + payload

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Hosted visitor sessions boot the optimized RELEASE kernel: ~2 s to the
# desktop (vs ~20 s debug) and the codecs decode an order of magnitude faster.
# Build with `cargo build --release` (deploy step) before restarting this agent.
KERNEL = os.path.join(ROOT, "target/aarch64-unknown-none/release/veil")
NOVNC = os.path.expanduser("~/server/novnc")
# websockify is not always on PATH (it's a pip --user install here); resolve
# a concrete path so the per-session websockify actually launches.
WEBSOCKIFY = (shutil.which("websockify")
              or os.path.expanduser("~/Library/Python/3.9/bin/websockify"))


def pump(a, b):
    """Splice two sockets bidirectionally until either closes (used to
    reverse-proxy a session's HTTP + WebSocket traffic to its websockify)."""
    socks = [a, b]
    try:
        while True:
            r, _, _ = select.select(socks, [], [], 180)
            if not r:
                break
            for s in r:
                try:
                    data = s.recv(65536)
                except OSError:
                    return
                if not data:
                    return
                try:
                    (b if s is a else a).sendall(data)
                except OSError:
                    return
    finally:
        for s in (a, b):
            try:
                s.close()
            except OSError:
                pass

LISTEN_PORT = 6090   # the port the live Cloudflare route already targets
MAX_SESSIONS = 20
VNC_BASE = 11        # QEMU -vnc :11 -> tcp 5911
WS_BASE = 6100       # websockify port per session
IDLE_TIMEOUT = 30 * 60

ADJECTIVES = [
    "crimson", "lucky", "silent", "golden", "azure", "brave", "clever",
    "swift", "amber", "cosmic", "dapper", "eager", "fuzzy", "gentle",
    "humble", "jolly", "keen", "lively", "mighty", "noble", "plucky",
    "quiet", "rusty", "sly", "tidy", "vivid", "witty", "zesty", "alpha",
    "beta", "scarlet", "violet", "teal",
]
ANIMALS = [
    "wombat", "moose", "otter", "falcon", "lynx", "heron", "badger",
    "marten", "gecko", "puffin", "raven", "ferret", "bison", "stoat",
    "ocelot", "tapir", "quokka", "ibis", "narwhal", "panther", "weasel",
    "owl", "fox", "hare", "vole", "shrew", "newt", "crane", "egret",
    "mantis", "koala", "civet", "dingo",
]


class Session:
    def __init__(self, sid, name, vnc, ws):
        self.sid = sid
        self.name = name
        self.vnc = vnc            # QEMU -vnc display number
        self.ws = ws              # websockify port
        self.disk = f"/tmp/veil-session-{sid}.img"
        self.uploads = f"/tmp/veil-uploads-{sid}"
        self.fifo = f"/tmp/veil-audio-{sid}.fifo"
        self.qmp = f"/tmp/veil-session-{sid}-qmp.sock"
        self.serial = f"/tmp/veil-session-{sid}-serial.log"
        self.qemu = None
        self.wsproc = None
        self.fifo_rd = None   # read-end fd; a thread drains it (see drain_fifo)
        self.audio_clients = set()       # browser audio WebSocket sockets
        self.audio_bytes = {}            # client sock -> total PCM bytes sent
        self.audio_ok_logged = False     # AUDIO_BROWSER_OK emitted once/session
        self.audio_lock = threading.Lock()
        self.closed = False
        self.last_active = time.time()
        self.booted = False

    def upload_count(self):
        try:
            return len(os.listdir(self.uploads))
        except OSError:
            return 0


class Manager:
    def __init__(self):
        self.sessions = {}        # sid -> Session
        self.lock = threading.Lock()

    # --- allocation ----------------------------------------------------
    def gen_username(self):
        active = {s.name for s in self.sessions.values()}
        for _ in range(200):
            name = f"{random.choice(ADJECTIVES)}_{random.choice(ANIMALS)}"
            if name not in active:
                return name
        # Extremely unlikely fallback: disambiguate with a digit.
        return f"{name}{random.randint(0, 9)}"

    def free_slot(self):
        used_vnc = {s.vnc for s in self.sessions.values()}
        used_ws = {s.ws for s in self.sessions.values()}
        for i in range(MAX_SESSIONS):
            vnc, ws = VNC_BASE + i, WS_BASE + i
            if vnc not in used_vnc and ws not in used_ws:
                return vnc, ws
        return None

    def new_session(self, sid=None):
        """Reserve a slot + username. None if at capacity."""
        with self.lock:
            self.reap_idle_locked()
            if len(self.sessions) >= MAX_SESSIONS:
                return None
            slot = self.free_slot()
            if slot is None:
                return None
            if sid is None:
                sid = "%08x" % random.getrandbits(32)
            s = Session(sid, self.gen_username(), slot[0], slot[1])
            self.sessions[sid] = s
            return s

    def get_or_create(self, sid):
        """Look up a session, creating one bound to `sid` if unknown (the
        landing page mints the id on GET /, but uploads/boot may arrive
        first in tests). None at capacity."""
        with self.lock:
            s = self.sessions.get(sid)
        return s or self.new_session(sid)

    # --- lifecycle -----------------------------------------------------
    def build_disk(self, s, username=None):
        cmd = [os.path.join(ROOT, "scripts/mkdisk.sh"), "--out", s.disk]
        # M27: the kernel's first-boot setup screen now collects the name,
        # so hosted disks ship without a baked USER.TXT (--no-user). The
        # --username override is kept only for the test harness.
        if username:
            cmd += ["--username", username]
        else:
            cmd += ["--no-user"]
        if os.path.isdir(s.uploads) and os.listdir(s.uploads):
            cmd += ["--extra-dir", s.uploads]
        subprocess.run(cmd, cwd=ROOT, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def broadcast_audio(self, s, pcm):
        """Send PCM to this session's browser audio clients (WS binary). Once a
        client has been sent >10 KB of PCM, log AUDIO_BROWSER_OK once — the
        proof that browser audio actually streams end to end (Task 1)."""
        if not pcm:
            return
        with s.audio_lock:
            clients = list(s.audio_clients)
        for off in range(0, len(pcm), AUDIO_CHUNK):
            payload = pcm[off:off + AUDIO_CHUNK]
            frame = ws_binary_frame(payload)
            for sock in clients:
                try:
                    sock.sendall(frame)
                except OSError:
                    with s.audio_lock:
                        s.audio_clients.discard(sock)
                        s.audio_bytes.pop(sock, None)
                    continue
                with s.audio_lock:
                    s.audio_bytes[sock] = s.audio_bytes.get(sock, 0) + len(payload)
                    if not s.audio_ok_logged and s.audio_bytes[sock] > 10240:
                        s.audio_ok_logged = True
                        print(f"AUDIO_BROWSER_OK session={s.sid} "
                              f"client_bytes={s.audio_bytes[sock]}", flush=True)

    def drain_fifo(self, s):
        """Continuously read the QEMU `wav` audiodev FIFO and forward the PCM
        to browser audio clients.

        THIS IS LOAD-BEARING, not just for browser audio: QEMU's wav backend
        does a *blocking* write to the FIFO, so if nobody drains it the 64 KB
        pipe fills mid-playback and QEMU's main loop blocks on write — which
        freezes the entire VM (VNC, timers, every device). Draining in-process
        here, for the whole session lifetime, guarantees that never happens
        (the old design leaned on a separate Node bridge being healthy)."""
        fd = s.fifo_rd
        if fd is None:
            return
        saw_data = False
        carry = b""
        while not s.closed:
            r, _, _ = select.select([fd], [], [], 0.5)
            if not r:
                continue
            try:
                chunk = os.read(fd, 65536)
            except BlockingIOError:
                continue
            except OSError:
                break
            if not chunk:
                # All writers closed (QEMU exited / not yet started). The
                # O_NONBLOCK reader stays valid; pause briefly and retry so a
                # late QEMU write-open still rendezvouses.
                if s.qemu and s.qemu.poll() is not None:
                    break
                time.sleep(0.05)
                continue
            # Strip QEMU's leading RIFF/WAVE header (up to and incl. the
            # `data` chunk marker + size) so clients get pure PCM. Draining
            # happens regardless — this only shapes what's forwarded.
            if not saw_data:
                carry += chunk
                i = carry.find(b"data")
                if i < 0:
                    carry = carry[-8:]
                    continue
                saw_data = True
                pcm = carry[i + 8:]
                carry = b""
            else:
                pcm = chunk
            if pcm:
                self.broadcast_audio(s, pcm)

    def spawn(self, s):
        """Build the disk and launch QEMU (+websockify) for session s."""
        if s.booted:
            return
        self.build_disk(s)
        # FIFO audio tap.  QEMU opens the write end of the FIFO at startup,
        # which blocks until a reader also opens it (POSIX named-pipe
        # rendezvous).  Open the read end here in O_NONBLOCK|O_RDONLY so
        # QEMU's open() returns immediately, then drain it from a thread for
        # the session's lifetime (drain_fifo) — see that method for why this
        # is required to keep the VM from freezing.
        try:
            if not os.path.exists(s.fifo):
                os.mkfifo(s.fifo)
        except OSError:
            pass
        try:
            s.fifo_rd = os.open(s.fifo, os.O_RDONLY | os.O_NONBLOCK)
        except OSError:
            s.fifo_rd = None
        if s.fifo_rd is not None:
            threading.Thread(target=self.drain_fifo, args=(s,), daemon=True).start()
        s.qemu = subprocess.Popen([
            "qemu-system-aarch64",
            "-machine", "virt", "-cpu", "cortex-a72", "-smp", "4", "-m", "512M",
            "-device", "virtio-gpu-device",
            "-global", "virtio-mmio.force-legacy=false",
            "-device", "ramfb",
            "-device", "virtio-keyboard-device",
            "-device", "virtio-tablet-device",
            "-drive", f"if=none,file={s.disk},format=raw,id=hd0",
            "-device", "virtio-blk-device,drive=hd0",
            "-netdev", "user,id=net0",
            "-device", "virtio-net-device,netdev=net0",
            # M26: chat relay on the host, reachable via the slirp gateway.
            "-fw_cfg", "name=opt/veil.relay,string=10.0.2.2:7778",
            # M37: skip the ~16 s codec regression self-tests on visitor boots
            # (the MP3/H.264 decoders are still wired into the apps).
            "-fw_cfg", "name=opt/veil.fastboot,string=1",
            # M28: wav backend writes PCM to the per-session FIFO tap.
            "-audiodev", f"wav,id=snd0,path={s.fifo}",
            "-device", "virtio-sound-device,audiodev=snd0",
            # M42 step 10 (multiplayer): share=ignore keeps the framebuffer open
            # to multiple simultaneous VNC clients (the second visitor on a
            # shared session link), so QEMU multiplexes both users' mouse/keyboard
            # into one desktop instead of disconnecting the first client.
            "-vnc", f"127.0.0.1:{s.vnc},share=ignore",
            "-qmp", f"unix:{s.qmp},server,nowait",
            "-serial", f"file:{s.serial}",
            "-no-reboot", "-semihosting",
            "-kernel", KERNEL,
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if os.path.isdir(NOVNC):
            try:
                s.wsproc = subprocess.Popen([
                    WEBSOCKIFY, "--web", NOVNC,
                    str(s.ws), f"127.0.0.1:{5900 + s.vnc}",
                ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            except OSError:
                # websockify absent (e.g. test box) — QEMU still serves VNC.
                print("session: websockify not found; VNC only", flush=True)
        # Wait for QEMU's VNC port to be ready before marking booted.
        # Without this the browser arrives before QEMU has opened the port,
        # websockify can't connect, and noVNC shows "failed to connect".
        vnc_tcp = 5900 + s.vnc
        deadline = time.time() + 15
        while time.time() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", vnc_tcp), timeout=0.5):
                    break
            except OSError:
                time.sleep(0.25)
        s.booted = True
        s.last_active = time.time()

    def kill(self, s):
        s.closed = True  # stop the drain thread
        with s.audio_lock:
            clients = list(s.audio_clients)
            s.audio_clients.clear()
        for sock in clients:
            try:
                sock.close()
            except OSError:
                pass
        for p in (s.qemu, s.wsproc):
            if p and p.poll() is None:
                try:
                    p.terminate()
                except ProcessLookupError:
                    pass
        if s.fifo_rd is not None:
            try:
                os.close(s.fifo_rd)
            except OSError:
                pass
        for path in (s.disk, s.fifo, s.qmp, s.serial):
            try:
                os.remove(path)
            except OSError:
                pass
        shutil.rmtree(s.uploads, ignore_errors=True)

    def reap_idle_locked(self):
        now = time.time()
        dead = [sid for sid, s in self.sessions.items()
                if now - s.last_active > IDLE_TIMEOUT
                or (s.qemu and s.qemu.poll() is not None)]
        for sid in dead:
            self.kill(self.sessions.pop(sid))

    def close(self, sid):
        with self.lock:
            s = self.sessions.pop(sid, None)
            if s:
                self.kill(s)

    def shutdown(self):
        with self.lock:
            for s in list(self.sessions.values()):
                self.kill(s)
            self.sessions.clear()


MGR = Manager()

FULL_PAGE = """<!doctype html><html><head><meta charset=utf-8>
<title>Veil OS — full</title><style>
body{background:#0b1018;color:#cdd6f4;font-family:monospace;text-align:center;padding-top:18vh}
h1{color:#89b4fa}</style></head><body>
<h1>Veil OS is at capacity</h1>
<p>All %d demo seats are busy. Try again in a moment.</p>
</body></html>""" % MAX_SESSIONS


class Handler(BaseHTTPRequestHandler):
    server_version = "veil-session-manager/1.0"

    def log_message(self, *a):
        pass

    def _send(self, code, body, ctype="text/html; charset=utf-8"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _redirect(self, loc):
        self.send_response(302)
        self.send_header("Location", loc)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        u = urlparse(self.path)
        if u.path in ("/", "/index.html"):
            return self.serve_landing()
        if u.path == "/close":
            sid = parse_qs(u.query).get("session", [""])[0]
            MGR.close(sid)
            return self._send(200, "closed")
        if u.path == "/healthz":
            return self._send(200, "ok\n", "text/plain")
        # M42 step 10 (multiplayer): a "Share Desktop" link for an existing
        # session. A second visitor opening /share?session=<id> joins the SAME
        # running QEMU (get_or_create reuses the session), and because QEMU's VNC
        # is share=ignore both clients see the same framebuffer and both mouse +
        # keyboard streams multiplex into the one desktop.
        if u.path == "/share":
            sid = parse_qs(u.query).get("session", [""])[0]
            with MGR.lock:
                exists = sid in MGR.sessions
            if not sid or not exists:
                return self._send(404, "no such session to share")
            join = f"/session/{sid}/vnc.html?path=session/{sid}/websockify&autoconnect=true&shared=true"
            return self._redirect(join)
        if u.path.startswith("/session/"):
            parts = u.path.split("/", 3)
            rest = parts[3] if len(parts) > 3 else ""
            if rest.split("?", 1)[0] == "audio":
                return self.audio_ws(parts[2])
            return self.proxy_session(u)
        self._send(404, "not found")

    def audio_ws(self, sid):
        """Upgrade /session/<sid>/audio to a WebSocket and register it for
        this session's PCM (the drain thread pushes binary frames)."""
        with MGR.lock:
            s = MGR.sessions.get(sid)
        if s is None or not s.booted:
            return self._send(404, "no such session")
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            return self._send(400, "expected websocket")
        self.connection.sendall((
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {ws_accept(key)}\r\n\r\n"
        ).encode())
        self.close_connection = True
        sock = self.connection
        with s.audio_lock:
            s.audio_clients.add(sock)
        s.last_active = time.time()
        try:
            # We only push audio; block until the client closes (discard any
            # frames it sends). recv returning b"" or erroring => gone.
            while not s.closed:
                if not sock.recv(4096):
                    break
        except OSError:
            pass
        finally:
            with s.audio_lock:
                s.audio_clients.discard(sock)
                s.audio_bytes.pop(sock, None)
            try:
                sock.close()
            except OSError:
                pass

    def proxy_session(self, u):
        """Reverse-proxy /session/<sid>/<rest> (noVNC static files and the
        WebSocket) to that session's private websockify port."""
        parts = u.path.split("/", 3)  # ['', 'session', sid, rest]
        if len(parts) < 3 or not parts[2]:
            return self._send(404, "bad session path")
        sid = parts[2]
        rest = parts[3] if len(parts) > 3 else ""
        with MGR.lock:
            s = MGR.sessions.get(sid)
        if s is None or not s.booted:
            return self._send(404, "no such session")
        s.last_active = time.time()
        path = "/" + rest + (("?" + u.query) if u.query else "")
        is_ws = self.headers.get("Upgrade", "").lower() == "websocket"
        try:
            up = socket.create_connection(("127.0.0.1", s.ws), timeout=10)
        except OSError:
            return self._send(502, "session backend unavailable")
        # Replay the request with the /session/<sid> prefix stripped. Force a
        # non-keepalive connection for plain HTTP so the splice ends on EOF;
        # WebSocket upgrades keep the connection open for bidirectional pump.
        lines = [f"{self.command} {path} HTTP/1.1"]
        for k, v in self.headers.items():
            if k.lower() == "connection" and not is_ws:
                continue
            lines.append(f"{k}: {v}")
        if not is_ws:
            lines.append("Connection: close")
        up.sendall(("\r\n".join(lines) + "\r\n\r\n").encode("latin1"))
        n = int(self.headers.get("Content-Length", "0") or 0)
        if n:
            up.sendall(self.rfile.read(n))
        self.close_connection = True
        pump(self.connection, up)

    def serve_landing(self):
        # M25 default: one-shot spawn + redirect. M30 overrides this with a
        # pre-boot upload page (scripts/landing.html); if that file exists,
        # serve it with a fresh session id injected.
        landing = os.path.join(ROOT, "scripts/landing.html")
        s = MGR.new_session()
        if s is None:
            return self._send(503, FULL_PAGE)
        if os.path.isfile(landing):
            page = open(landing).read().replace("__SESSION__", s.sid)
            return self._send(200, page)
        # No landing page: boot immediately (legacy single-click demo).
        try:
            MGR.spawn(s)
        except Exception as e:  # noqa
            MGR.close(s.sid)
            return self._send(500, f"spawn failed: {html.escape(str(e))}")
        self._redirect(f"/session/{s.sid}/vnc.html?path=session/{s.sid}/websockify&autoconnect=true")

    def do_POST(self):
        u = urlparse(self.path)
        sid = parse_qs(u.query).get("session", [""])[0]
        if not sid:
            return self._send(400, "missing session")
        s = MGR.get_or_create(sid)
        if s is None:
            return self._send(503, FULL_PAGE)
        if u.path == "/upload":
            return self.handle_upload(s)
        if u.path == "/boot":
            return self.handle_boot(s)
        self._send(404, "not found")

    def handle_upload(self, s):
        # Accept .png/.wav files (<=4MB each, 5 max per session) from a
        # multipart/form-data body; stage them in the session upload dir.
        ctype = self.headers.get("Content-Type", "")
        if "multipart/form-data" not in ctype or "boundary=" not in ctype:
            return self._send(400, "expected multipart/form-data")
        boundary = ctype.split("boundary=", 1)[1].strip().strip('"')
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        os.makedirs(s.uploads, exist_ok=True)
        saved, errors = [], []
        for part in body.split(("--" + boundary).encode()):
            part = part.strip(b"\r\n")
            if not part or part == b"--" or b"\r\n\r\n" not in part:
                continue
            head, _, content = part.partition(b"\r\n\r\n")
            content = content.rstrip(b"\r\n")
            m = re.search(r'filename="([^"]*)"', head.decode("latin1"))
            if not m or not m.group(1):
                continue
            fname = os.path.basename(m.group(1))
            ext = fname.rsplit(".", 1)[-1].lower() if "." in fname else ""
            if ext not in ("png", "wav", "gif", "jpg", "jpeg"):
                errors.append(f"{fname}: only .png/.jpg/.wav/.gif")
            elif len(content) > 4 * 1024 * 1024:
                errors.append(f"{fname}: over 4MB")
            elif s.upload_count() + len(saved) >= 5:
                errors.append("max 5 files")
            else:
                with open(os.path.join(s.uploads, fname), "wb") as fh:
                    fh.write(content)
                saved.append(fname)
        s.last_active = time.time()
        self._send(200, json.dumps({"saved": saved, "errors": errors}),
                   "application/json")

    def handle_boot(self, s):
        try:
            MGR.spawn(s)
        except Exception as e:  # noqa
            import traceback
            traceback.print_exc()
            sys.stdout.flush()
            MGR.close(s.sid)
            return self._send(500, f"spawn failed: {html.escape(str(e))}")
        self._redirect(f"/session/{s.sid}/vnc.html?path=session/{s.sid}/websockify&autoconnect=true")


class DualStackServer(ThreadingHTTPServer):
    """Bind IPv6 + IPv4 (cloudflared resolves `localhost` to [::1], so an
    IPv4-only bind gets 'connection refused' through the tunnel)."""
    address_family = socket.AF_INET6
    daemon_threads = True

    def server_bind(self):
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        try:
            self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        except OSError:
            pass
        super().server_bind()


def serve():
    httpd = DualStackServer(("::", LISTEN_PORT), Handler)

    def bye(*a):
        # Kill child QEMUs, then exit hard: httpd.shutdown() would deadlock
        # if called from this signal handler (it runs on the serving thread).
        MGR.shutdown()
        os._exit(0)
    signal.signal(signal.SIGTERM, bye)
    signal.signal(signal.SIGINT, bye)
    print(f"veil session manager on 127.0.0.1:{LISTEN_PORT} "
          f"(max {MAX_SESSIONS} sessions)")
    try:
        httpd.serve_forever()
    finally:
        MGR.shutdown()


def selftest():
    ok = True
    # Unique usernames under load.
    names = set()
    for _ in range(MAX_SESSIONS):
        s = MGR.new_session()
        assert s is not None, "ran out of slots early"
        assert s.name not in names, f"duplicate username {s.name}"
        names.add(s.name)
    assert MGR.new_session() is None, "exceeded MAX_SESSIONS"
    print(f"ok   {len(names)} unique usernames, capacity enforced")
    # Port allocation is collision-free.
    vncs = {s.vnc for s in MGR.sessions.values()}
    wss = {s.ws for s in MGR.sessions.values()}
    assert len(vncs) == MAX_SESSIONS and len(wss) == MAX_SESSIONS
    print("ok   VNC + websockify ports unique")
    # Disk build path is wired correctly (build one real image).
    s = next(iter(MGR.sessions.values()))
    try:
        MGR.build_disk(s, username="selftest_fox")
        size = os.path.getsize(s.disk)
        assert size > 1_000_000
        print(f"ok   built session disk ({size} bytes) via mkdisk.sh")
    except Exception as e:  # noqa
        ok = False
        print(f"FAIL disk build: {e}")
    finally:
        MGR.shutdown()
    print("SELFTEST PASS" if ok else "SELFTEST FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        selftest()
    else:
        serve()
