#!/usr/bin/env node
// M28 proof client: connect to the audio bridge WebSocket and confirm real
// PCM streams through. Reads binary frames until it has collected >= NEED
// bytes that are predominantly non-zero (actual tone, not silence), then
// prints AUDIO_STREAM_OK and exits 0. Times out (exit 1) otherwise.
//
// Usage: ws_probe.js <port> <session> [need-bytes] [timeout-ms]

const net = require("net");
const crypto = require("crypto");

const PORT = parseInt(process.argv[2] || "6092", 10);
const SESSION = process.argv[3] || "test";
const NEED = parseInt(process.argv[4] || "4096", 10);
const TIMEOUT = parseInt(process.argv[5] || "5000", 10);

const key = crypto.randomBytes(16).toString("base64");
const sock = net.connect(PORT, "127.0.0.1", () => {
  sock.write(
    `GET /?session=${SESSION} HTTP/1.1\r\nHost: localhost\r\n` +
    "Upgrade: websocket\r\nConnection: Upgrade\r\n" +
    `Sec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\n\r\n`
  );
});

let handshook = false;
let buf = Buffer.alloc(0);
let pcm = 0, nonzero = 0;

const fail = (msg) => { console.error("PROBE FAIL:", msg); process.exit(1); };
const timer = setTimeout(() => fail(`only ${pcm} PCM bytes in ${TIMEOUT}ms`), TIMEOUT);

function onPayload(p) {
  pcm += p.length;
  for (const b of p) if (b) nonzero++;
  // Require enough bytes AND that they're mostly real signal (>25% non-zero).
  if (pcm >= NEED && nonzero > NEED / 4) {
    clearTimeout(timer);
    console.log(`AUDIO_STREAM_OK ${pcm} bytes (${nonzero} non-zero)`);
    process.exit(0);
  }
}

function parseFrames() {
  while (buf.length >= 2) {
    const op = buf[0] & 0x0f;
    let len = buf[1] & 0x7f;
    let off = 2;
    if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
    else if (len === 127) { if (buf.length < 10) return; len = buf.readUInt32BE(6); off = 10; }
    const masked = buf[1] & 0x80;
    const need = off + (masked ? 4 : 0) + len;
    if (buf.length < need) return;
    let payload = buf.subarray(off + (masked ? 4 : 0), need);
    if (masked) {
      const mask = buf.subarray(off, off + 4);
      payload = Buffer.from(payload);
      for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
    }
    buf = buf.subarray(need);
    if (op === 0x8) fail("server closed");
    if (op === 0x2 || op === 0x0) onPayload(payload);
  }
}

sock.on("data", (d) => {
  if (!handshook) {
    buf = Buffer.concat([buf, d]);
    const i = buf.indexOf("\r\n\r\n");
    if (i < 0) return;
    if (!/101/.test(buf.subarray(0, i).toString())) fail("no 101 upgrade");
    handshook = true;
    buf = buf.subarray(i + 4);
  } else {
    buf = Buffer.concat([buf, d]);
  }
  parseFrames();
});
sock.on("error", (e) => fail(e.message));
sock.on("close", () => fail("socket closed"));
