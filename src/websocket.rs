//! From-scratch WebSocket (RFC 6455) client over our TCP/TLS stack. Implements
//! the HTTP Upgrade handshake (with SHA-1 + base64 for the Sec-WebSocket-Accept
//! check), and masked client→server framing / unmasked server→client decoding
//! for text, binary, ping/pong and close frames. `ws://` runs over a plain TCP
//! `net::Handle`; `wss://` runs over a `tls::TlsConn` — both via the `Stream`
//! trait. No external crates.

use crate::{net, timer, tls};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The WebSocket GUID appended to the client key before hashing (RFC 6455 §1.3).
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// ---- SHA-1 ----------------------------------------------------------------

pub fn sha1(msg: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let ml = (msg.len() as u64) * 8;
    let mut data = Vec::from(msg);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&ml.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

// ---- base64 ----------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Server-side accept value for a client key (also used by our loopback server).
pub fn accept_key(client_key: &str) -> String {
    let mut buf = Vec::from(client_key.as_bytes());
    buf.extend_from_slice(WS_GUID.as_bytes());
    base64_encode(&sha1(&buf))
}

// ---- Stream abstraction (TCP or TLS) --------------------------------------

pub trait Stream {
    fn write_all(&mut self, data: &[u8]);
    /// Read some bytes (blocking up to ~`ms` of silence). Empty = timed out.
    fn read_some(&mut self, ms: u64) -> Vec<u8>;
    fn close(&mut self);
}

impl Stream for Box<dyn Stream> {
    fn write_all(&mut self, data: &[u8]) {
        (**self).write_all(data)
    }
    fn read_some(&mut self, ms: u64) -> Vec<u8> {
        (**self).read_some(ms)
    }
    fn close(&mut self) {
        (**self).close()
    }
}

pub struct TcpStream(pub net::Handle);

impl Stream for TcpStream {
    fn write_all(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let n = net::tcp_write(self.0, data);
            data = &data[n..];
            if !data.is_empty() {
                crate::scheduler::yield_now();
            }
        }
    }
    fn read_some(&mut self, ms: u64) -> Vec<u8> {
        let deadline = timer::ticks() + ms / 20 + 1;
        let mut tmp = [0u8; 2048];
        loop {
            match net::tcp_read(self.0, &mut tmp) {
                net::TcpRead::Data(n) => return Vec::from(&tmp[..n]),
                net::TcpRead::Eof => return Vec::new(),
                net::TcpRead::Empty => {
                    if timer::ticks() > deadline {
                        return Vec::new();
                    }
                    crate::scheduler::yield_now();
                }
            }
        }
    }
    fn close(&mut self) {
        net::tcp_close(self.0);
    }
}

pub struct TlsStream(pub tls::TlsConn);

impl Stream for TlsStream {
    fn write_all(&mut self, data: &[u8]) {
        self.0.write(data);
    }
    fn read_some(&mut self, ms: u64) -> Vec<u8> {
        self.0.read(timer::ticks() + ms / 20 + 1).unwrap_or_default()
    }
    fn close(&mut self) {
        self.0.close();
    }
}

// ---- WebSocket framing -----------------------------------------------------

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    Other(u8),
}

impl Opcode {
    fn from(b: u8) -> Opcode {
        match b & 0x0f {
            0x0 => Opcode::Continuation,
            0x1 => Opcode::Text,
            0x2 => Opcode::Binary,
            0x8 => Opcode::Close,
            0x9 => Opcode::Ping,
            0xa => Opcode::Pong,
            o => Opcode::Other(o),
        }
    }
    fn code(self) -> u8 {
        match self {
            Opcode::Continuation => 0x0,
            Opcode::Text => 0x1,
            Opcode::Binary => 0x2,
            Opcode::Close => 0x8,
            Opcode::Ping => 0x9,
            Opcode::Pong => 0xa,
            Opcode::Other(o) => o,
        }
    }
}

pub struct WebSocket<S: Stream> {
    stream: S,
    rxbuf: Vec<u8>,
    mask_seed: u32,
    masked: bool, // client frames are masked; server frames are not
    pub open: bool,
}

impl<S: Stream> WebSocket<S> {
    /// Wrap an already-upgraded server stream (frames it sends are unmasked).
    pub fn server(stream: S, leftover: Vec<u8>) -> WebSocket<S> {
        WebSocket { stream, rxbuf: leftover, mask_seed: read_cycles() as u32 | 1, masked: false, open: true }
    }

    /// Build a frame (FIN=1) for `payload` with `op`, masked iff `self.masked`.
    fn encode(&mut self, op: Opcode, payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.push(0x80 | op.code()); // FIN + opcode
        let len = payload.len();
        let mb = if self.masked { 0x80 } else { 0x00 };
        if len < 126 {
            f.push(mb | len as u8);
        } else if len < 65536 {
            f.push(mb | 126);
            f.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            f.push(mb | 127);
            f.extend_from_slice(&(len as u64).to_be_bytes());
        }
        if self.masked {
            // 4-byte mask key from a churning seed (any 32-bit value is legal).
            self.mask_seed = self.mask_seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = self.mask_seed.to_be_bytes();
            f.extend_from_slice(&key);
            for (i, b) in payload.iter().enumerate() {
                f.push(b ^ key[i % 4]);
            }
        } else {
            f.extend_from_slice(payload);
        }
        f
    }

    pub fn send_text(&mut self, s: &str) {
        let frame = self.encode(Opcode::Text, s.as_bytes());
        self.stream.write_all(&frame);
    }

    pub fn send_binary(&mut self, data: &[u8]) {
        let frame = self.encode(Opcode::Binary, data);
        self.stream.write_all(&frame);
    }

    pub fn send_ping(&mut self, data: &[u8]) {
        let frame = self.encode(Opcode::Ping, data);
        self.stream.write_all(&frame);
    }

    fn send_pong(&mut self, data: &[u8]) {
        let frame = self.encode(Opcode::Pong, data);
        self.stream.write_all(&frame);
    }

    pub fn close(&mut self) {
        if self.open {
            let frame = self.encode(Opcode::Close, &[]);
            self.stream.write_all(&frame);
            self.open = false;
        }
        self.stream.close();
    }

    /// Receive one application frame (text/binary), auto-handling ping→pong and
    /// close. Returns (opcode, payload) or None on timeout/closed. Reassembles
    /// fragmented messages. `ms` bounds total wait.
    pub fn recv(&mut self, ms: u64) -> Option<(Opcode, Vec<u8>)> {
        let deadline = timer::ticks() + ms / 20 + 1;
        let mut msg: Vec<u8> = Vec::new();
        let mut msg_op = Opcode::Text;
        loop {
            // Ensure we have a full frame header + length + payload buffered.
            let frame = loop {
                if let Some(f) = self.try_parse_frame() {
                    break f;
                }
                if timer::ticks() > deadline {
                    return None;
                }
                let more = self.stream.read_some(ms.min(200));
                if more.is_empty() {
                    if timer::ticks() > deadline {
                        return None;
                    }
                    continue;
                }
                self.rxbuf.extend_from_slice(&more);
            };
            let (fin, op, payload) = frame;
            match op {
                Opcode::Ping => {
                    self.send_pong(&payload);
                    continue;
                }
                Opcode::Pong => {
                    return Some((Opcode::Pong, payload));
                }
                Opcode::Close => {
                    self.open = false;
                    return Some((Opcode::Close, payload));
                }
                Opcode::Continuation => {
                    msg.extend_from_slice(&payload);
                }
                other => {
                    msg_op = other;
                    msg.extend_from_slice(&payload);
                }
            }
            if fin {
                return Some((msg_op, msg));
            }
        }
    }

    /// Parse one frame out of `rxbuf` if fully buffered (server frames are
    /// unmasked). Removes the consumed bytes. Returns (fin, opcode, payload).
    fn try_parse_frame(&mut self) -> Option<(bool, Opcode, Vec<u8>)> {
        let b = &self.rxbuf;
        if b.len() < 2 {
            return None;
        }
        let fin = b[0] & 0x80 != 0;
        let op = Opcode::from(b[0]);
        let masked = b[1] & 0x80 != 0;
        let mut len = (b[1] & 0x7f) as usize;
        let mut hdr = 2;
        if len == 126 {
            if b.len() < 4 {
                return None;
            }
            len = u16::from_be_bytes([b[2], b[3]]) as usize;
            hdr = 4;
        } else if len == 127 {
            if b.len() < 10 {
                return None;
            }
            len = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]) as usize;
            hdr = 10;
        }
        let mask = if masked {
            if b.len() < hdr + 4 {
                return None;
            }
            let m = [b[hdr], b[hdr + 1], b[hdr + 2], b[hdr + 3]];
            hdr += 4;
            Some(m)
        } else {
            None
        };
        if b.len() < hdr + len {
            return None;
        }
        let mut payload = Vec::from(&b[hdr..hdr + len]);
        if let Some(m) = mask {
            for (i, x) in payload.iter_mut().enumerate() {
                *x ^= m[i % 4];
            }
        }
        self.rxbuf.drain(0..hdr + len);
        Some((fin, op, payload))
    }
}

/// Open a WebSocket to `url` (ws:// or wss://). Performs the TCP/TLS connect and
/// the HTTP Upgrade handshake, verifying Sec-WebSocket-Accept.
pub fn connect(url: &str) -> Option<WebSocket<Box<dyn Stream>>> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("ws://") {
        (false, r)
    } else {
        return None;
    };
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(if secure { 443 } else { 80 })),
        None => (host_port, if secure { 443 } else { 80 }),
    };

    // A 16-byte client key, base64'd, derived from the cycle counter.
    let seed = read_cycles();
    let mut nonce = [0u8; 16];
    for (i, n) in nonce.iter_mut().enumerate() {
        *n = (seed.rotate_left((i * 7) as u32) ^ (i as u64).wrapping_mul(0x9E3779B9)) as u8;
    }
    let key = base64_encode(&nonce);
    let req = alloc::format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );

    let mut stream: Box<dyn Stream> = if secure {
        let conn = tls::tls_connect(host, port)?;
        Box::new(TlsStream(conn))
    } else {
        let ip = if is_loopback(host) {
            net::local_ip()?
        } else {
            net::dns_resolve(host)?
        };
        let h = net::tcp_connect(ip, port)?;
        Box::new(TcpStream(h))
    };
    stream.write_all(req.as_bytes());

    // Read the 101 handshake response (header block).
    let mut resp = Vec::new();
    let deadline = timer::ticks() + 200;
    while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
        if timer::ticks() > deadline {
            return None;
        }
        let more = stream.read_some(200);
        if more.is_empty() {
            crate::scheduler::yield_now();
            continue;
        }
        resp.extend_from_slice(&more);
        if resp.len() > 8192 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&resp);
    if !head.contains("101") {
        crate::kprintln!("WS: handshake not 101 for {url}");
        return None;
    }
    let want = accept_key(&key);
    let got = head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("sec-websocket-accept")).map(|(_, v)| v.trim().to_string()));
    if got.as_deref() != Some(want.as_str()) {
        crate::kprintln!("WS: accept mismatch (got {:?}, want {want})", got);
        return None;
    }
    // Any bytes after the header terminator are the first frames.
    let leftover = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| resp[p + 4..].to_vec())
        .unwrap_or_default();
    kprintln_ok();
    Some(WebSocket { stream, rxbuf: leftover, mask_seed: seed as u32 | 1, masked: true, open: true })
}

fn is_loopback(host: &str) -> bool {
    host == "veil" || host == "localhost" || host == "127.0.0.1" || host.is_empty()
}

fn read_cycles() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
    v
}

use core::sync::atomic::{AtomicBool, Ordering};
static HANDSHAKE_LOGGED: AtomicBool = AtomicBool::new(false);
fn kprintln_ok() {
    if !HANDSHAKE_LOGGED.swap(true, Ordering::Relaxed) {
        crate::kprintln!("WS: Upgrade handshake complete (Sec-WebSocket-Accept verified)");
    }
}

/// Self-test: SHA-1 + base64 known-answer (the RFC 6455 example), proving the
/// handshake math without needing a live server.
pub fn selftest() {
    // RFC 6455 §1.3 example: key "dGhlIHNhbXBsZSBub25jZQ==" -> accept
    // "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".
    let acc = accept_key("dGhlIHNhbXBsZSBub25jZQ==");
    let sha_ok = base64_encode(&sha1(b"abc")) == "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=";
    if acc == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" && sha_ok {
        crate::kprintln!("WS_PROTO_OK: SHA-1 + base64 + Sec-WebSocket-Accept match the RFC 6455 vectors");
    } else {
        crate::kprintln!("WS_PROTO_FAIL: accept={acc} sha_ok={sha_ok}");
    }
}
