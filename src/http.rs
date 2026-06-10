//! Network services, run as a preemptively-scheduled kernel task:
//! a TCP echo service on :7777 (the M14 nc proof) and an HTTP/1.1 server
//! on :80 serving the FAT16 filesystem (M15). Connections are handled one
//! at a time — every response is Connection: close and small, so browsers'
//! parallel fetches just queue behind each other in accept order.

use crate::{fs, kprintln, net, scheduler, timer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

const ECHO_PORT: u16 = 7777;
const HTTP_PORT: u16 = 80;
const REQ_TIMEOUT: u64 = 250; // 5 s at the 50 Hz tick

static HTTP_OK: AtomicBool = AtomicBool::new(false);

pub fn services_task() {
    net::tcp_listen(ECHO_PORT);
    net::tcp_listen(HTTP_PORT);
    kprintln!("SRV: tcp echo on :{ECHO_PORT}, http on :{HTTP_PORT}");
    loop {
        if let Some(h) = net::tcp_accept(ECHO_PORT) {
            echo_session(h);
        }
        if let Some(h) = net::tcp_accept(HTTP_PORT) {
            http_session(h);
        }
        scheduler::yield_now();
    }
}

/// Queue all of `data`, yielding while the send buffer is full. Returns
/// early (claiming success) if the connection dies — callers can't help.
fn write_all(h: net::Handle, mut data: &[u8]) {
    while !data.is_empty() {
        let n = net::tcp_write(h, data);
        data = &data[n..];
        if !data.is_empty() {
            scheduler::yield_now();
        }
    }
}

/// Read until `pat` shows up (returning everything read), EOF, `cap`
/// bytes, or `timeout` ticks of silence.
fn read_until(h: net::Handle, pat: &[u8], cap: usize, timeout: u64) -> Option<Vec<u8>> {
    let deadline = timer::ticks() + timeout;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        match net::tcp_read(h, &mut tmp) {
            net::TcpRead::Data(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(pat.len()).any(|w| w == pat) {
                    return Some(buf);
                }
                if buf.len() > cap {
                    return None;
                }
            }
            net::TcpRead::Empty => {
                if timer::ticks() > deadline {
                    return None;
                }
                scheduler::yield_now();
            }
            net::TcpRead::Eof => {
                return buf.windows(pat.len()).any(|w| w == pat).then_some(buf);
            }
        }
    }
}

/// M14 proof partner: greet (server->client bytes), echo one line back
/// (client->server->client), then close our side first so the capture
/// shows a full orderly FIN handshake.
fn echo_session(h: net::Handle) {
    if let Some((ip, port)) = net::tcp_remote(h) {
        kprintln!("ECHO: session from {}:{port}", net::fmt_ip(&ip));
    }
    write_all(h, b"VEIL TCP ECHO\n");
    if let Some(line) = read_until(h, b"\n", 4096, REQ_TIMEOUT) {
        write_all(h, b"echo: ");
        write_all(h, &line);
    }
    net::tcp_close(h);
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e) {
        Some("HTM") | Some("HTML") => "text/html",
        Some("CSS") => "text/css",
        Some("PNG") => "image/png",
        Some("BMP") => "image/bmp",
        Some("TXT") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// "/style.css?x=1" -> "STYLE.CSS"; "/" -> "INDEX.HTM"; bad paths -> None.
fn resolve(path: &str) -> Option<String> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    let name = path.strip_prefix('/')?;
    let name = if name.is_empty() { "INDEX.HTM" } else { name };
    if name.contains('/') || name.len() > 12 {
        return None;
    }
    let mut up = String::from(name);
    up.make_ascii_uppercase();
    Some(up)
}

fn http_session(h: net::Handle) {
    let Some(req) = read_until(h, b"\r\n\r\n", 8192, REQ_TIMEOUT) else {
        net::tcp_close(h);
        return;
    };
    let line = core::str::from_utf8(&req)
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("");
    let mut parts = line.split(' ');
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));

    let (status, ctype, body): (&str, &str, Vec<u8>) = if method != "GET" {
        ("405 Method Not Allowed", "text/html", Vec::from(&b"<html><body><h1>405</h1></body></html>"[..]))
    } else {
        match resolve(path).and_then(|name| fs::read_file(&name).map(|d| (name, d))) {
            Some((name, data)) => ("200 OK", content_type(&name), data),
            None => (
                "404 Not Found",
                "text/html",
                Vec::from(&b"<html><body><h1>404 - not on this disk</h1></body></html>"[..]),
            ),
        }
    };

    let headers = format!(
        "HTTP/1.1 {status}\r\nServer: veil/0.1\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_all(h, headers.as_bytes());
    write_all(h, &body);
    net::tcp_close(h);
    kprintln!("HTTP: {method} {path} -> {} ({} bytes)", &status[..3], body.len());
    if status.starts_with("200") && !HTTP_OK.swap(true, Ordering::Relaxed) {
        kprintln!("HTTP_OK: served a file off the FAT16 disk over our TCP");
        kprintln!("M15_OK");
    }
}
