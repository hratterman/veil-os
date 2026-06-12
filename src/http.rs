//! Network services, run as a preemptively-scheduled kernel task:
//! a TCP echo service on :7777 (the M14 nc proof) and an HTTP/1.1 server
//! on :80 serving the FAT16 filesystem (M15). Connections are handled one
//! at a time — every response is Connection: close and small, so browsers'
//! parallel fetches just queue behind each other in accept order.

use crate::{fs, kprintln, net, scheduler, timer};
use alloc::format;
use alloc::string::{String, ToString};
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
        Some("JPG") | Some("JPEG") => "image/jpeg",
        Some("GIF") => "image/gif",
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
    let Some(mut req) = read_until(h, b"\r\n\r\n", 8192, REQ_TIMEOUT) else {
        net::tcp_close(h);
        return;
    };
    let head_str = core::str::from_utf8(&req).unwrap_or("").to_string();
    let line = head_str.lines().next().unwrap_or("");
    let mut parts = line.split(' ');
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());

    // Lowercased header lookups (Content-Length, Cookie).
    let hdr = |name: &str| -> Option<String> {
        let lname = name.to_ascii_lowercase();
        head_str.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().eq_ignore_ascii_case(&lname)).then(|| v.trim().to_string())
        })
    };

    // For POST, pull the body (after the header terminator), reading more if the
    // first segment didn't carry it all.
    let mut body_bytes = Vec::new();
    if method == "POST" {
        if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
            body_bytes = req.split_off(pos + 4);
        }
        let want = hdr("content-length").and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
        let mut tmp = [0u8; 1024];
        let mut spins = 0;
        while body_bytes.len() < want && spins < 200 {
            match net::tcp_read(h, &mut tmp) {
                net::TcpRead::Data(n) => {
                    body_bytes.extend_from_slice(&tmp[..n]);
                    spins = 0;
                }
                net::TcpRead::Empty => {
                    spins += 1;
                    scheduler::yield_now();
                }
                net::TcpRead::Eof => break,
            }
        }
    }
    let body_str = String::from_utf8_lossy(&body_bytes).into_owned();

    // --- special endpoints (form/cookie demo for the M40 form test) ----------
    let p = path.split(['?', '#']).next().unwrap_or("");
    if p == "/login" && method == "POST" {
        let user = form_field(&body_str, "username").unwrap_or_default();
        kprintln!("HTTP: login POST user={user:?} ({} body bytes)", body_bytes.len());
        let resp = format!(
            "HTTP/1.1 302 Found\r\nServer: veil/0.1\r\nSet-Cookie: session=veil-{user}; Path=/\r\nLocation: /welcome\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        write_all(h, resp.as_bytes());
        net::tcp_close(h);
        return;
    }
    if p == "/welcome" {
        let cookie = hdr("cookie").unwrap_or_default();
        let who = form_field(&cookie.replace("; ", "&"), "session");
        let page = match &who {
            Some(s) => format!(
                "<html><head><link rel=stylesheet href=style.css></head><body><h1>LOGGED IN</h1><p>Cookie session = {s}. Welcome back.</p><p><a href=/index.htm>Home</a></p></body></html>"
            ),
            None => String::from(
                "<html><body><h1>NOT LOGGED IN</h1><p>No session cookie was sent.</p></body></html>",
            ),
        };
        kprintln!("HTTP: /welcome cookie={cookie:?} -> {}", if who.is_some() { "LOGGED IN" } else { "NOT LOGGED IN" });
        send(h, "200 OK", "text/html", page.as_bytes());
        return;
    }

    if p == "/echo" {
        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
        kprintln!("HTTP: /echo query={query:?}");
        let page = format!(
            "<html><head><link rel=stylesheet href=style.css></head><body><h1>SUBMITTED</h1><pre>{query}</pre></body></html>"
        );
        send(h, "200 OK", "text/html", page.as_bytes());
        return;
    }

    let (status, ctype, body): (&str, &str, Vec<u8>) = if method != "GET" {
        ("405 Method Not Allowed", "text/html", Vec::from(&b"<html><body><h1>405</h1></body></html>"[..]))
    } else {
        match resolve(&path).and_then(|name| fs::read_file(&name).map(|d| (name, d))) {
            Some((name, data)) => ("200 OK", content_type(&name), data),
            None => (
                "404 Not Found",
                "text/html",
                Vec::from(&b"<html><body><h1>404 - not on this disk</h1></body></html>"[..]),
            ),
        }
    };
    send(h, status, ctype, &body);
    kprintln!("HTTP: {method} {path} -> {} ({} bytes)", &status[..3], body.len());
    if status.starts_with("200") && !HTTP_OK.swap(true, Ordering::Relaxed) {
        kprintln!("HTTP_OK: served a file off the FAT16 disk over our TCP");
        kprintln!("M15_OK");
    }
}

/// Send a simple response.
fn send(h: net::Handle, status: &str, ctype: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nServer: veil/0.1\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_all(h, headers.as_bytes());
    write_all(h, body);
    net::tcp_close(h);
}

/// Pull `name`'s value from an `application/x-www-form-urlencoded` string.
fn form_field(body: &str, name: &str) -> Option<String> {
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == name {
                return Some(v.replace('+', " "));
            }
        }
    }
    None
}
