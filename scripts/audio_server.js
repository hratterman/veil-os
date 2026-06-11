#!/usr/bin/env node
// Veil OS browser-audio bridge (M28). Taps each hosted session's QEMU `wav`
// audiodev FIFO (/tmp/veil-audio-<id>.fifo) and forwards the raw 16-bit
// stereo 44100 Hz PCM to that session's browser over a WebSocket as binary
// frames. No npm deps — Node built-ins only.
//
//   browser: wss://audio.henryratterman.com/?session=<id>
//   tap:     /tmp/veil-audio-<id>.fifo  (QEMU -audiodev wav,path=...)
//
// Runs as LaunchAgent com.veil.audio on :6092. Cloudflare ingress
// audio.henryratterman.com -> localhost:6092 (WebSocket upgrade enabled).
//
// Usage: audio_server.js [port]   (default 6092)

const http = require("http");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const PORT = parseInt(process.argv[2] || "6092", 10);
const TMP = "/tmp";
const FIFO_RE = /^veil-audio-(.+)\.fifo$/;
const WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const CHUNK = 4096; // ~23 ms at 44100 Hz stereo 16-bit

// session id -> { clients:Set<socket>, opened:bool, carry:Buffer, sawData:bool }
const sessions = new Map();

function sess(id) {
  let s = sessions.get(id);
  if (!s) {
    s = { clients: new Set(), opened: false, carry: Buffer.alloc(0), sawData: false };
    sessions.set(id, s);
  }
  return s;
}

// Strip the leading 44-byte RIFF/WAVE header (QEMU's wav backend writes one
// up front) so the browser receives pure PCM. We scan for the `data` chunk.
function stripHeader(s, buf) {
  if (s.sawData) return buf;
  s.carry = Buffer.concat([s.carry, buf]);
  const i = s.carry.indexOf("data");
  if (i < 0) {
    // Keep the tail (in case `data` straddles two reads) and emit nothing.
    if (s.carry.length > 8) s.carry = s.carry.subarray(s.carry.length - 8);
    return Buffer.alloc(0);
  }
  s.sawData = true;
  const pcm = s.carry.subarray(i + 8); // skip 'data' + 4-byte size
  s.carry = Buffer.alloc(0);
  return pcm;
}

function wsFrame(payload) {
  const len = payload.length;
  let head;
  if (len < 126) {
    head = Buffer.from([0x82, len]);
  } else if (len < 65536) {
    head = Buffer.alloc(4);
    head[0] = 0x82; head[1] = 126; head.writeUInt16BE(len, 2);
  } else {
    head = Buffer.alloc(10);
    head[0] = 0x82; head[1] = 127; head.writeUInt32BE(0, 2); head.writeUInt32BE(len, 6);
  }
  return Buffer.concat([head, payload]);
}

function broadcast(s, pcm) {
  if (!pcm.length) return;
  for (let off = 0; off < pcm.length; off += CHUNK) {
    const frame = wsFrame(pcm.subarray(off, off + CHUNK));
    for (const sock of s.clients) {
      if (sock.writable) sock.write(frame);
    }
  }
}

// Tap a session's FIFO: open it for reading (rendezvous with QEMU's write
// open at VM boot), drain forever, forward PCM to WS clients.
function tap(id) {
  const s = sess(id);
  if (s.opened) return;
  s.opened = true;
  const fifo = path.join(TMP, `veil-audio-${id}.fifo`);
  const open = () => {
    const rs = fs.createReadStream(fifo);
    rs.on("data", (buf) => broadcast(s, stripHeader(s, buf)));
    rs.on("error", () => setTimeout(reopenSoon, 500));
    rs.on("end", () => setTimeout(reopenSoon, 200));
  };
  const reopenSoon = () => {
    if (!fs.existsSync(fifo)) { s.opened = false; return; } // session gone
    s.sawData = false; s.carry = Buffer.alloc(0);
    open();
  };
  open();
}

// Discover new session FIFOs (the session manager mkfifo's them at boot).
function scan() {
  let names = [];
  try { names = fs.readdirSync(TMP); } catch (_) {}
  for (const n of names) {
    const m = FIFO_RE.exec(n);
    if (m) tap(m[1]);
  }
}
setInterval(scan, 500);
scan();

const server = http.createServer((req, res) => {
  res.writeHead(200, { "Content-Type": "text/plain" });
  res.end("veil audio bridge\n");
});

server.on("upgrade", (req, socket) => {
  const key = req.headers["sec-websocket-key"];
  if (!key) { socket.destroy(); return; }
  const url = new URL(req.url, "http://x");
  const id = url.searchParams.get("session");
  const accept = crypto.createHash("sha1").update(key + WS_GUID).digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
    "Upgrade: websocket\r\nConnection: Upgrade\r\n" +
    `Sec-WebSocket-Accept: ${accept}\r\n\r\n`
  );
  if (!id) { socket.end(); return; }
  const s = sess(id);
  s.clients.add(socket);
  tap(id);
  const drop = () => s.clients.delete(socket);
  socket.on("close", drop);
  socket.on("error", drop);
  // Drain (and ignore) any client->server frames; we only push audio.
  socket.on("data", () => {});
});

server.listen(PORT, "127.0.0.1", () =>
  console.log(`veil audio bridge on 127.0.0.1:${PORT}`));
