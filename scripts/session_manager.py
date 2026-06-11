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

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KERNEL = os.path.join(ROOT, "target/aarch64-unknown-none/debug/veil")
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

    def spawn(self, s):
        """Build the disk and launch QEMU (+websockify) for session s."""
        if s.booted:
            return
        self.build_disk(s)
        # FIFO audio tap for the bridge (com.veil.audio reads it; the open
        # rendezvous unblocks QEMU's write-open at boot).
        try:
            if not os.path.exists(s.fifo):
                os.mkfifo(s.fifo)
        except OSError:
            pass
        s.qemu = subprocess.Popen([
            "qemu-system-aarch64",
            "-machine", "virt", "-cpu", "cortex-a72", "-m", "512M",
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
            # M28: wav backend writes PCM to the per-session FIFO tap.
            "-audiodev", f"wav,id=snd0,path={s.fifo}",
            "-device", "virtio-sound-device,audiodev=snd0",
            "-vnc", f"127.0.0.1:{s.vnc}",
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
        for p in (s.qemu, s.wsproc):
            if p and p.poll() is None:
                try:
                    p.terminate()
                except ProcessLookupError:
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
        if u.path.startswith("/session/"):
            return self.proxy_session(u)
        self._send(404, "not found")

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
            if ext not in ("png", "wav"):
                errors.append(f"{fname}: only .png/.wav")
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
