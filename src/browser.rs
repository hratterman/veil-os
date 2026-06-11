//! M16: the on-OS browser. Fetches pages from the OS's own HTTP server
//! over the OS's own TCP stack (loopback — both halves of every
//! connection are ours), parses, lays out and paints them in a window,
//! and navigates when a link is clicked.
//!
//! THE SUPPORTED GRAMMAR (deliberately bounded; the line is held here):
//!
//! HTML elements: html, head (skipped), body, h1-h6, p, a (href), ul, ol,
//!   li, img (src, PNG only), div, span, br, pre, link (stylesheet ref).
//!   Unknown tags render as inline containers. Entities: amp lt gt quot
//!   apos nbsp #NNN. Comments and doctypes are skipped. li/p implicitly
//!   close on a sibling opener; br/img/link/meta/hr are void.
//!
//! CSS: `tag`, `.class` and `tag.class` selectors (comma groups allowed;
//!   descendants/ids/pseudo-classes are ignored). Properties: color,
//!   background-color, font-size (rounded to whole multiples of the 16px
//!   font: 1x, 2x, ...), margin / padding (single px value, or per-side
//!   -top/-right/-bottom/-left), width (px), display (block|inline|none).
//!   Colors: #rgb, #rrggbb, and a small name table. class > tag
//!   specificity, later rules win within a tier. color/font-size inherit.
//!
//! Layout: block boxes stack vertically (no margin collapsing); inline
//!   content flows into bottom-aligned line boxes with word wrap. pre is
//!   verbatim, unwrapped. Documents render into a full-page buffer
//!   (capped at MAX_DOC_H rows), so scrolling is a row copy.

use crate::fb::Framebuffer;
use crate::wm::Window;
use crate::{css, html, kprintln, net, png, scheduler, timer, tls};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

pub const TOPBAR: usize = 20;
const MAX_DOC_H: usize = 3000;
const FETCH_TIMEOUT: u64 = 500; // 10 s at the 50 Hz tick
const BAR_BG: u32 = 0xffc8_ccd4;
const BAR_TEXT: u32 = 0xff20_2830;

static M16_DONE: AtomicBool = AtomicBool::new(false);

pub struct BrowserState {
    pub path: String,
    page: Vec<u32>, // page_w * doc_h, the fully rendered document
    page_w: usize,
    doc_h: usize,
    links: Vec<LinkBox>,
    scroll: usize,
    page_bg: u32,
    history: Vec<String>, // previously-visited paths (newest last), max 20
    img_cache: Vec<(String, png::Image)>, // decoded images by URL, LRU, cap 10
}

struct LinkBox {
    x: isize,
    y: isize,
    w: isize,
    h: isize,
    href: String,
}

impl BrowserState {
    pub fn new() -> BrowserState {
        BrowserState {
            path: String::from("/"),
            page: vec![0xffff_ffff; 1],
            page_w: 1,
            doc_h: 1,
            links: Vec::new(),
            scroll: 0,
            page_bg: 0xffff_ffff,
            history: Vec::new(),
            img_cache: Vec::new(),
        }
    }
}

static EXT_IMG_DONE: AtomicBool = AtomicBool::new(false);
const IMG_CACHE_CAP: usize = 10;

// --- HTTP client over our own TCP, via loopback --------------------------------

fn write_all(h: net::Handle, mut data: &[u8]) {
    while !data.is_empty() {
        let n = net::tcp_write(h, data);
        data = &data[n..];
        if !data.is_empty() {
            scheduler::yield_now();
        }
    }
}

/// Index just past the end of the HTTP header block (`\r\n\r\n`), if present.
fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length(head_lower: &str) -> Option<usize> {
    head_lower.lines().find_map(|l| {
        let (n, v) = l.split_once(':')?;
        (n.trim() == "content-length").then(|| v.trim().parse().ok())?
    })
}

/// True once a chunked body contains its terminating 0-length chunk.
fn chunked_complete(body: &[u8]) -> bool {
    let mut i = 0;
    loop {
        let mut j = i;
        while j + 1 < body.len() && !(body[j] == b'\r' && body[j + 1] == b'\n') {
            j += 1;
        }
        if j + 1 >= body.len() {
            return false; // size line not fully received
        }
        let line = core::str::from_utf8(&body[i..j]).unwrap_or("");
        let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16);
        match size {
            Ok(0) => return true,            // terminator chunk
            Ok(sz) => i = j + 2 + sz + 2,    // size CRLF + data + CRLF
            Err(_) => return false,
        }
        if i > body.len() {
            return false;
        }
    }
}

/// Has a full HTTP/1.1 response arrived? Uses Content-Length or the chunked
/// terminator; returns false (keep reading until close/deadline) when neither
/// is present. This is what stops a keep-alive server — which never sends EOF
/// — from hanging the reader (and thus the whole desktop) forever.
fn response_complete(buf: &[u8]) -> bool {
    let Some(hend) = header_end(buf) else {
        return false;
    };
    let head = core::str::from_utf8(&buf[..hend]).unwrap_or("").to_ascii_lowercase();
    if head.contains("transfer-encoding:") && head.contains("chunked") {
        return chunked_complete(&buf[hend..]);
    }
    if let Some(cl) = content_length(&head) {
        return buf.len() >= hend + cl;
    }
    false
}

static HTTP_READ_DONE: AtomicBool = AtomicBool::new(false);

/// One-time proof that the response terminated on a parsed length (not on the
/// old block-until-EOF path that hung on keep-alive servers).
fn note_bounded_read() {
    if !HTTP_READ_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("HTTP_READ_OK");
    }
}

/// Read an HTTP response over plain TCP. Returns as soon as the response is
/// complete (Content-Length / chunked), on EOF, when `cap` is exceeded, after
/// `idle` ticks with no data, or after a hard `hard` total-time backstop —
/// whichever comes first, so it can never block indefinitely.
fn read_http(h: net::Handle, cap: usize, idle: u64, hard: u64) -> Vec<u8> {
    let hard_deadline = timer::ticks() + hard;
    let mut idle_deadline = timer::ticks() + idle;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match net::tcp_read(h, &mut tmp) {
            net::TcpRead::Data(n) => {
                buf.extend_from_slice(&tmp[..n]);
                idle_deadline = timer::ticks() + idle;
                if buf.len() > cap {
                    return buf;
                }
                if response_complete(&buf) {
                    note_bounded_read();
                    return buf;
                }
            }
            net::TcpRead::Empty => {
                if timer::ticks() > idle_deadline {
                    return buf;
                }
                scheduler::yield_now();
            }
            net::TcpRead::Eof => return buf,
        }
        if timer::ticks() > hard_deadline {
            return buf;
        }
    }
}

/// True for a fully-qualified URL pointing somewhere off our own machine.
fn is_external(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    (u.starts_with("http://") || u.starts_with("https://"))
        && !u.contains("//127.0.0.1")
        && !u.contains("//10.0.2.")
        && !u.contains("//veil")
        && !u.contains("//localhost")
}

fn is_https(url: &str) -> bool {
    let u = url.get(..8).unwrap_or("").to_ascii_lowercase();
    u == "https://"
}

static TLS_OK_DONE: AtomicBool = AtomicBool::new(false);

/// Decode HTTP/1.1 `Transfer-Encoding: chunked` bodies (Cloudflare et al. use
/// it). A no-op if the body isn't actually chunked.
fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let mut j = i;
        while j + 1 < body.len() && !(body[j] == b'\r' && body[j + 1] == b'\n') {
            j += 1;
        }
        let line = core::str::from_utf8(&body[i..j]).unwrap_or("");
        let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).unwrap_or(0);
        i = j + 2;
        if size == 0 {
            break;
        }
        let end = (i + size).min(body.len());
        out.extend_from_slice(&body[i..end]);
        i = end + 2; // skip the chunk's trailing CRLF
    }
    out
}

/// Split a raw HTTP/1.1 response into (status, content-type, body), decoding a
/// chunked body if present.
fn parse_response(resp: &[u8], path: &str) -> Option<(u32, String, Vec<u8>)> {
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(&resp[..split]).unwrap_or("");
    let status: u32 = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let ctype = head
        .lines()
        .find_map(|l| {
            let (name, val) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-type").then(|| String::from(val.trim()))
        })
        .unwrap_or_default();
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    let raw = &resp[split + 4..];
    let body = if chunked { dechunk(raw) } else { raw.to_vec() };
    kprintln!("BROWSER: GET {path} -> {status} {ctype} ({} bytes)", body.len());
    Some((status, ctype, body))
}

/// Direct TLS 1.3 fetch of an `https://` URL (no proxy). Parses host/port/path,
/// does the handshake, sends the GET, reads the decrypted response.
fn tls_get(url: &str) -> Option<(u32, String, Vec<u8>)> {
    let rest = url.get(8..)?; // strip "https://"
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(443)),
        None => (hostport, 443u16),
    };
    let mut conn = tls::tls_connect(host, port)?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: VeilOS\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    );
    conn.write(req.as_bytes());
    let deadline = timer::ticks() + 600;
    let mut resp = Vec::new();
    while let Some(chunk) = conn.read(deadline) {
        resp.extend_from_slice(&chunk);
        // Stop as soon as the response is complete — real HTTPS servers keep
        // the connection open (no close_notify), so waiting for EOF would hang.
        if resp.len() > (1 << 20) {
            break;
        }
        if response_complete(&resp) {
            note_bounded_read();
            break;
        }
    }
    conn.close();
    let parsed = parse_response(&resp, url)?;
    if parsed.0 == 200 && !TLS_OK_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("TLS_OK");
    }
    Some(parsed)
}

/// GET `path`. Local paths ("/page.htm") hit our own HTTP server on loopback;
/// `https://` URLs use the from-scratch TLS 1.3 stack directly; other external
/// `http://` URLs go through the host proxy at 10.0.2.2:7779.
fn http_get(path: &str) -> Option<(u32, String, Vec<u8>)> {
    if is_https(path) {
        if let Some(r) = tls_get(path) {
            return Some(r);
        }
        kprintln!("BROWSER: direct TLS failed for {path}, falling back to proxy");
    }
    let external = is_external(path);
    let (ip, port, timeout) = if external {
        ([10, 0, 2, 2], 7779u16, 1500) // proxy fetch can take a while
    } else {
        (net::local_ip()?, 80u16, FETCH_TIMEOUT)
    };
    for attempt in 0..2 {
        if attempt > 0 {
            for _ in 0..10 {
                scheduler::yield_now();
            }
        }
        let Some(h) = net::tcp_connect(ip, port) else { continue };
        let host = if external { "proxy" } else { "veil" };
        let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        write_all(h, req.as_bytes());
        // idle = stall timeout; hard = absolute backstop so a keep-alive peer
        // that never closes can't hang the desktop. response_complete() returns
        // us promptly once Content-Length / chunked says the body is done.
        let resp = read_http(h, 1 << 20, timeout, timeout * 4);
        net::tcp_close(h);
        if let Some(r) = parse_response(&resp, path) {
            return Some(r);
        }
    }
    kprintln!("BROWSER: GET {path} failed (no response)");
    None
}

/// "page2.htm" -> "/page2.htm"; absolute paths and full external URLs pass
/// through (the proxy keeps query strings, so don't strip those for URLs).
fn resolve_href(href: &str) -> String {
    if is_external(href) {
        return String::from(href);
    }
    let href = href.split(['#', '?']).next().unwrap_or("");
    if href.starts_with('/') {
        String::from(href)
    } else {
        format!("/{href}")
    }
}

// --- style ----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Display {
    Block,
    Inline,
    None,
}

#[derive(Clone)]
struct Style {
    color: u32,
    bg: Option<u32>,
    scale: usize,
    margin: [isize; 4],  // top right bottom left
    padding: [isize; 4],
    display: Display,
    width: Option<isize>,
    underline: bool,
    pre: bool,
}

const ROOT_STYLE: Style = Style {
    color: 0xff10_1418,
    bg: None,
    scale: 1,
    margin: [0; 4],
    padding: [0; 4],
    display: Display::Block,
    width: None,
    underline: false,
    pre: false,
};

fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let d = |c: u8| (c as char).to_digit(16);
        let h = hex.as_bytes();
        return match h.len() {
            3 => Some(
                0xff00_0000
                    | d(h[0])? * 0x11 << 16
                    | d(h[1])? * 0x11 << 8
                    | d(h[2])? * 0x11,
            ),
            6 => Some(
                0xff00_0000
                    | (d(h[0])? << 4 | d(h[1])?) << 16
                    | (d(h[2])? << 4 | d(h[3])?) << 8
                    | (d(h[4])? << 4 | d(h[5])?),
            ),
            _ => None,
        };
    }
    match v.to_ascii_lowercase().as_str() {
        "black" => Some(0xff00_0000),
        "white" => Some(0xffff_ffff),
        "red" => Some(0xffe0_3030),
        "green" => Some(0xff30_a050),
        "blue" => Some(0xff30_60e0),
        "yellow" => Some(0xffe0_d030),
        "orange" => Some(0xffe0_a040),
        "gray" | "grey" => Some(0xff80_8890),
        _ => None,
    }
}

fn parse_px(v: &str) -> Option<isize> {
    v.trim().trim_end_matches("px").trim().parse().ok()
}

fn apply_decl(s: &mut Style, prop: &str, val: &str) {
    match prop {
        "color" => {
            if let Some(c) = parse_color(val) {
                s.color = c;
            }
        }
        "background-color" => s.bg = parse_color(val),
        "font-size" => {
            if let Some(px) = parse_px(val) {
                s.scale = (((px + 8) / 16).max(1) as usize).min(4);
            }
        }
        "margin" => {
            if let Some(px) = parse_px(val) {
                s.margin = [px; 4];
            }
        }
        "margin-top" => s.margin[0] = parse_px(val).unwrap_or(s.margin[0]),
        "margin-right" => s.margin[1] = parse_px(val).unwrap_or(s.margin[1]),
        "margin-bottom" => s.margin[2] = parse_px(val).unwrap_or(s.margin[2]),
        "margin-left" => s.margin[3] = parse_px(val).unwrap_or(s.margin[3]),
        "padding" => {
            if let Some(px) = parse_px(val) {
                s.padding = [px; 4];
            }
        }
        "padding-top" => s.padding[0] = parse_px(val).unwrap_or(s.padding[0]),
        "padding-right" => s.padding[1] = parse_px(val).unwrap_or(s.padding[1]),
        "padding-bottom" => s.padding[2] = parse_px(val).unwrap_or(s.padding[2]),
        "padding-left" => s.padding[3] = parse_px(val).unwrap_or(s.padding[3]),
        "width" => s.width = parse_px(val),
        "display" => {
            s.display = match val.trim() {
                "none" => Display::None,
                "inline" => Display::Inline,
                "block" => Display::Block,
                _ => s.display,
            }
        }
        _ => {}
    }
}

/// UA defaults for `node`, then matching stylesheet rules (tag tier, then
/// class tier; later rules win within a tier). color/font-size/underline
/// inherit from the parent.
fn resolve(sheet: &[css::Rule], node: &html::Node, inherited: &Style) -> Style {
    let tag = node.tag().unwrap_or("");
    let class = node.attr("class");
    let mut s = Style {
        color: inherited.color,
        bg: None,
        scale: inherited.scale,
        margin: [0; 4],
        padding: [0; 4],
        display: Display::Inline,
        width: None,
        underline: inherited.underline,
        pre: inherited.pre,
    };
    match tag {
        "html" => s.display = Display::Block,
        "body" => {
            s.display = Display::Block;
            s.margin = [8; 4];
        }
        "div" => s.display = Display::Block,
        "p" => {
            s.display = Display::Block;
            s.margin[0] = 8;
            s.margin[2] = 8;
        }
        "h1" => {
            s.display = Display::Block;
            s.scale = 2;
            s.margin[0] = 16;
            s.margin[2] = 16;
        }
        "h2" | "h3" | "h4" | "h5" | "h6" => {
            s.display = Display::Block;
            s.margin[0] = 12;
            s.margin[2] = 12;
        }
        "ul" | "ol" => {
            s.display = Display::Block;
            s.margin[0] = 8;
            s.margin[2] = 8;
            s.padding[3] = 28;
        }
        "li" => {
            s.display = Display::Block;
            s.margin[2] = 2;
        }
        "pre" => {
            s.display = Display::Block;
            s.margin[0] = 8;
            s.margin[2] = 8;
            s.pre = true;
        }
        "a" => {
            s.color = 0xff20_50c0;
            s.underline = true;
        }
        "table" | "tr" | "td" | "th" => s.display = Display::Block,
        "head" | "title" | "script" | "style" | "meta" | "link" => s.display = Display::None,
        _ => {} // span, img, unknown: inline containers
    }
    for tier in 0..2 {
        for r in sheet {
            if r.rank() == tier && r.matches(tag, class) {
                for (p, v) in &r.decls {
                    apply_decl(&mut s, p, v);
                }
            }
        }
    }
    s
}

// --- layout ---------------------------------------------------------------------

enum Item {
    Rect { x: isize, y: isize, w: isize, h: isize, color: u32 },
    Text { x: isize, y: isize, s: String, color: u32, scale: usize },
    Image { x: isize, y: isize, idx: usize },
}

enum Frag {
    Word { s: String, color: u32, scale: usize, link: Option<String>, underline: bool },
    Space { scale: usize },
    Img { idx: usize, w: isize, h: isize },
    Br,
}

fn frag_w(f: &Frag) -> isize {
    match f {
        Frag::Word { s, scale, .. } => s.len() as isize * 8 * *scale as isize,
        Frag::Space { scale } => 8 * *scale as isize,
        Frag::Img { w, .. } => *w,
        Frag::Br => 0,
    }
}

fn frag_h(f: &Frag) -> isize {
    match f {
        Frag::Word { scale, .. } | Frag::Space { scale } => 16 * *scale as isize,
        Frag::Img { h, .. } => *h,
        Frag::Br => 16,
    }
}

struct Ctx<'a> {
    sheet: &'a [css::Rule],
    imgs: &'a [(String, png::Image)],
    items: Vec<Item>,
    links: Vec<LinkBox>,
    img_spots: Vec<(usize, isize, isize)>, // (imgs idx, x, y) for the proof log
}

/// Whitespace-collapsing text -> word/space frags.
fn collect_text(t: &str, style: &Style, link: &Option<String>, buf: &mut Vec<Frag>) {
    let mut word = String::new();
    let flush = |word: &mut String, buf: &mut Vec<Frag>| {
        if !word.is_empty() {
            buf.push(Frag::Word {
                s: core::mem::take(word),
                color: style.color,
                scale: style.scale,
                link: link.clone(),
                underline: style.underline,
            });
        }
    };
    for c in t.chars() {
        if c.is_ascii_whitespace() {
            flush(&mut word, buf);
            if !matches!(buf.last(), Some(Frag::Space { .. }) | None) {
                buf.push(Frag::Space { scale: style.scale });
            }
        } else {
            word.push(c);
        }
    }
    flush(&mut word, buf);
}

/// Flatten an inline subtree into frags.
fn collect_inline(
    ctx: &Ctx,
    node: &html::Node,
    style: &Style,
    link: &Option<String>,
    buf: &mut Vec<Frag>,
) {
    match node {
        html::Node::Text(t) => collect_text(t, style, link, buf),
        html::Node::Element { tag, children, .. } => {
            let link = match tag.as_str() {
                "a" => node.attr("href").map(resolve_href).or_else(|| link.clone()),
                _ => link.clone(),
            };
            match tag.as_str() {
                "br" => buf.push(Frag::Br),
                "img" => {
                    let src = node.attr("src").map(resolve_href).unwrap_or_default();
                    match ctx.imgs.iter().position(|(s, _)| *s == src) {
                        Some(idx) => {
                            let img = &ctx.imgs[idx].1;
                            buf.push(Frag::Img {
                                idx,
                                w: img.w as isize,
                                h: img.h as isize,
                            });
                        }
                        // Not decoded (failed fetch / non-PNG): render nothing.
                        None => {}
                    }
                }
                _ => {
                    for c in children {
                        let cs = match c {
                            html::Node::Element { .. } => resolve(ctx.sheet, c, style),
                            html::Node::Text(_) => style.clone(),
                        };
                        if cs.display != Display::None {
                            collect_inline(ctx, c, &cs, &link, buf);
                        }
                    }
                }
            }
        }
    }
}

/// Place one line box, bottom-aligning its frags. Returns the new y.
fn place_line(ctx: &mut Ctx, line: Vec<(isize, Frag)>, x: isize, y: isize) -> isize {
    let lh = line.iter().map(|(_, f)| frag_h(f)).max().unwrap_or(16);
    for (dx, f) in line {
        let fh = frag_h(&f);
        let fy = y + lh - fh;
        match f {
            Frag::Word { s, color, scale, link, underline } => {
                let fw = s.len() as isize * 8 * scale as isize;
                if underline {
                    ctx.items.push(Item::Rect {
                        x: x + dx,
                        y: fy + 16 * scale as isize - scale as isize,
                        w: fw,
                        h: scale as isize,
                        color,
                    });
                }
                if let Some(href) = link {
                    ctx.links.push(LinkBox {
                        x: x + dx,
                        y: fy,
                        w: fw,
                        h: 16 * scale as isize,
                        href,
                    });
                }
                ctx.items.push(Item::Text { x: x + dx, y: fy, s, color, scale });
            }
            Frag::Img { idx, .. } => {
                ctx.img_spots.push((idx, x + dx, fy));
                ctx.items.push(Item::Image { x: x + dx, y: fy, idx });
            }
            Frag::Space { .. } | Frag::Br => {}
        }
    }
    y + lh
}

/// Word-wrap buffered frags into line boxes at x with width w.
fn flush_inline(ctx: &mut Ctx, buf: &mut Vec<Frag>, x: isize, w: isize, mut y: isize) -> isize {
    if buf.is_empty() {
        return y;
    }
    let mut line: Vec<(isize, Frag)> = Vec::new();
    let mut cx = 0isize;
    for f in core::mem::take(buf) {
        if matches!(f, Frag::Br) {
            if line.is_empty() {
                y += 16;
            } else {
                y = place_line(ctx, core::mem::take(&mut line), x, y);
            }
            cx = 0;
            continue;
        }
        if matches!(f, Frag::Space { .. }) && line.is_empty() {
            continue; // never start a line with a space
        }
        let fw = frag_w(&f);
        if cx + fw > w && !line.is_empty() {
            y = place_line(ctx, core::mem::take(&mut line), x, y);
            cx = 0;
            if matches!(f, Frag::Space { .. }) {
                continue;
            }
        }
        line.push((cx, f));
        cx += fw;
    }
    if !line.is_empty() {
        y = place_line(ctx, line, x, y);
    }
    y
}

static TABLE_DONE: AtomicBool = AtomicBool::new(false);

/// Lay out a `<table>`: equal-width columns, each cell a block container,
/// 1px borders between cells. No rowspan/colspan. Returns the y below it.
fn layout_table(
    ctx: &mut Ctx,
    node: &html::Node,
    style: &Style,
    x: isize,
    w: isize,
    mut y: isize,
) -> isize {
    const LINE: u32 = 0xff58_6068;
    const PAD: isize = 4;
    // Collect rows (anywhere under the table, so an implicit <tbody> is fine).
    let mut trs: Vec<&html::Node> = Vec::new();
    node.find_all("tr", &mut trs);
    if trs.is_empty() {
        return y;
    }
    let row_cells: Vec<Vec<&html::Node>> = trs
        .iter()
        .map(|tr| {
            tr.children()
                .iter()
                .filter(|c| matches!(c.tag(), Some("td") | Some("th")))
                .collect()
        })
        .collect();
    let ncols = row_cells.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let col_w = (w / ncols as isize).max(24);
    let table_w = col_w * ncols as isize;
    let table_top = y;
    for cells in &row_cells {
        let row_top = y;
        let mut row_bottom = row_top + 16; // a min row height
        for (j, cell) in cells.iter().enumerate().take(ncols) {
            let cx = x + j as isize * col_w + PAD;
            let cw = (col_w - 2 * PAD).max(8);
            let cstyle = resolve(ctx.sheet, cell, style);
            let bottom = layout_children(ctx, cell, &cstyle, cx, cw, row_top + PAD);
            row_bottom = row_bottom.max(bottom + PAD);
        }
        y = row_bottom;
        // Column separators down this row, then a rule under it.
        for j in 0..=ncols {
            ctx.items.push(Item::Rect { x: x + j as isize * col_w, y: row_top, w: 1, h: y - row_top, color: LINE });
        }
        ctx.items.push(Item::Rect { x, y, w: table_w, h: 1, color: LINE });
    }
    ctx.items.push(Item::Rect { x, y: table_top, w: table_w, h: 1, color: LINE });
    if !TABLE_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("TABLE_OK");
    }
    y + 8
}

/// Lay out one block element. Returns the y below it (margins included).
fn layout_block(
    ctx: &mut Ctx,
    node: &html::Node,
    inherited: &Style,
    x: isize,
    w: isize,
    mut y: isize,
    marker: Option<String>,
) -> isize {
    let style = resolve(ctx.sheet, node, inherited);
    if style.display == Display::None {
        return y;
    }
    y += style.margin[0];
    let bx = x + style.margin[3];
    let bw = style
        .width
        .unwrap_or(w - style.margin[3] - style.margin[1])
        .max(16);
    if node.tag() == Some("table") {
        return layout_table(ctx, node, &style, bx, bw, y) + style.margin[2];
    }
    let bg_at = ctx.items.len();
    let cx = bx + style.padding[3];
    let cw = (bw - style.padding[3] - style.padding[1]).max(16);
    let top = y;
    let mut cy = y + style.padding[0];

    if let Some(m) = marker {
        let mw = m.len() as isize * 8;
        ctx.items.push(Item::Text {
            x: (cx - mw).max(0),
            y: cy,
            s: m,
            color: style.color,
            scale: 1,
        });
    }

    if style.pre {
        // Verbatim: no wrap, no whitespace collapsing.
        let mut text = String::new();
        node.text(&mut text);
        for line in text.trim_matches('\n').lines() {
            ctx.items.push(Item::Text {
                x: cx,
                y: cy,
                s: String::from(line),
                color: style.color,
                scale: style.scale,
            });
            cy += 16 * style.scale as isize;
        }
    } else {
        cy = layout_children(ctx, node, &style, cx, cw, cy);
    }

    let bottom = cy + style.padding[2];
    if let Some(bg) = style.bg {
        ctx.items.insert(
            bg_at,
            Item::Rect { x: bx, y: top, w: bw, h: bottom - top, color: bg },
        );
    }
    bottom + style.margin[2]
}

/// Lay out an element's children: inline runs flow, blocks recurse.
fn layout_children(
    ctx: &mut Ctx,
    node: &html::Node,
    style: &Style,
    x: isize,
    w: isize,
    mut y: isize,
) -> isize {
    let mut buf: Vec<Frag> = Vec::new();
    let mut ol_count = 0usize;
    for child in node.children() {
        let cstyle = match child {
            html::Node::Element { .. } => resolve(ctx.sheet, child, style),
            html::Node::Text(_) => style.clone(),
        };
        if cstyle.display == Display::None {
            continue;
        }
        let is_block = matches!(child, html::Node::Element { .. })
            && cstyle.display == Display::Block;
        if !is_block {
            collect_inline(ctx, child, &cstyle, &None, &mut buf);
            continue;
        }
        y = flush_inline(ctx, &mut buf, x, w, y);
        let marker = match (node.tag(), child.tag()) {
            (Some("ul"), Some("li")) => Some(String::from("* ")),
            (Some("ol"), Some("li")) => {
                ol_count += 1;
                Some(format!("{ol_count}. "))
            }
            _ => None,
        };
        y = layout_block(ctx, child, style, x, w, y, marker);
    }
    flush_inline(ctx, &mut buf, x, w, y)
}

// --- navigation -------------------------------------------------------------------

/// Fetch `path` + its stylesheet + images, lay the page out at the
/// window's content width, render it into the page buffer, and repaint.
static HISTORY_DONE: AtomicBool = AtomicBool::new(false);

/// Navigate back to the previously-visited path, if any.
pub fn back(win: &mut Window) -> bool {
    let prev = match &mut win.app {
        crate::wm::App::Browser(st) => st.history.pop(),
        _ => return false,
    };
    let Some(p) = prev else { return false };
    kprintln!("BROWSER: back to {p}");
    navigate(win, &p, false);
    if !HISTORY_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("HISTORY_OK");
    }
    true
}

pub fn navigate(win: &mut Window, path: &str, by_click: bool) {
    let path = resolve_href(path);
    let was_external = is_external(&path);
    let path_for_log = path.clone();
    kprintln!("BROWSER: navigating to {path}");
    // A user navigation (link click / typed URL) pushes the current page onto
    // the history stack so the back button can return to it.
    if by_click {
        if let crate::wm::App::Browser(st) = &mut win.app {
            let old = st.path.clone();
            if st.history.last() != Some(&old) {
                st.history.push(old);
                if st.history.len() > 20 {
                    st.history.remove(0);
                }
            }
        }
    }
    let Some((status, ctype, body)) = http_get(&path) else {
        render_message(win, &path, "fetch failed: no response from server");
        return;
    };
    if !ctype.to_ascii_lowercase().contains("text/html") {
        render_message(win, &path, &format!("not html: {ctype} ({status})"));
        return;
    }
    let doc = html::parse(&String::from_utf8_lossy(&body));

    // Stylesheets: <link rel="stylesheet" href=...> anywhere in the doc.
    let mut sheet: Vec<css::Rule> = Vec::new();
    let mut link_nodes = Vec::new();
    doc.find_all("link", &mut link_nodes);
    for l in link_nodes {
        let rel = l.attr("rel").unwrap_or_default();
        if !rel.eq_ignore_ascii_case("stylesheet") {
            continue;
        }
        let Some(href) = l.attr("href") else { continue };
        if let Some((200, _, css_body)) = http_get(&resolve_href(href)) {
            sheet.extend(css::parse(&String::from_utf8_lossy(&css_body)));
        }
    }

    // Images: fetch + decode each unique PNG src, served by the persistent LRU
    // cache so navigating back doesn't re-fetch. https -> TLS, http -> proxy,
    // relative/absolute -> loopback (all via http_get). Non-PNG (JPEG/WebP/SVG)
    // is skipped silently — no [img] placeholder.
    let mut cache = match &mut win.app {
        crate::wm::App::Browser(st) => core::mem::take(&mut st.img_cache),
        _ => Vec::new(),
    };
    let mut imgs: Vec<(String, png::Image)> = Vec::new();
    let mut img_nodes = Vec::new();
    doc.find_all("img", &mut img_nodes);
    for node in img_nodes {
        let Some(src) = node.attr("src").map(resolve_href) else { continue };
        if imgs.iter().any(|(s, _)| *s == src) {
            continue;
        }
        // Cache hit: move-to-front (LRU) and reuse the decoded image.
        if let Some(pos) = cache.iter().position(|(s, _)| *s == src) {
            let entry = cache.remove(pos);
            imgs.push((entry.0.clone(), entry.1.clone()));
            cache.insert(0, entry);
            continue;
        }
        if let Some((200, _, data)) = http_get(&src) {
            match png::decode(&data) {
                Some(img) => {
                    kprintln!("BROWSER: decoded {src} ({}x{} px)", img.w, img.h);
                    if is_external(&src) && !EXT_IMG_DONE.swap(true, Ordering::Relaxed) {
                        kprintln!("EXT_IMG_OK");
                    }
                    cache.insert(0, (src.clone(), img.clone()));
                    cache.truncate(IMG_CACHE_CAP);
                    imgs.push((src, img));
                }
                None => kprintln!("BROWSER: {src} is not a PNG (skipped, no placeholder)"),
            }
        }
    }
    // Return the (possibly grown) cache to the window for the next navigation.
    if let crate::wm::App::Browser(st) = &mut win.app {
        st.img_cache = cache;
    }

    // Layout at the window's content width.
    let view_w = win.cw;
    let body_node = doc.find("body").unwrap_or(&doc);
    let body_style = resolve(&sheet, body_node, &ROOT_STYLE);
    let page_bg = body_style.bg.unwrap_or(0xffff_ffff);
    let mut ctx = Ctx {
        sheet: &sheet,
        imgs: &imgs,
        items: Vec::new(),
        links: Vec::new(),
        img_spots: Vec::new(),
    };
    let end_y = layout_block(&mut ctx, body_node, &ROOT_STYLE, 0, view_w as isize, 0, None);
    let doc_h = (end_y.max(1) as usize).min(MAX_DOC_H);
    if end_y as usize > MAX_DOC_H {
        kprintln!("BROWSER: document truncated at {MAX_DOC_H}px (was {end_y})");
    }

    // Paint the whole document into the page buffer.
    let mut page = vec![page_bg; view_w * doc_h];
    let pfb = unsafe { Framebuffer::new(page.as_mut_ptr(), view_w, doc_h, view_w * 4) };
    for item in &ctx.items {
        match item {
            &Item::Rect { x, y, w, h, color } => {
                if w > 0 && h > 0 {
                    pfb.fill_rect(x.max(0) as usize, y.max(0) as usize, w as usize, h as usize, color);
                }
            }
            Item::Text { x, y, s, color, scale } => {
                if *y >= 0 {
                    pfb.draw_string_scaled((*x).max(0) as usize, *y as usize, s, *color, *scale);
                }
            }
            &Item::Image { x, y, idx } => {
                let img = &imgs[idx].1;
                pfb.blit(x, y, &img.pixels, img.w, img.h);
            }
        }
    }

    // Proof breadcrumbs: where things ended up, in document coordinates.
    for &(idx, x, y) in &ctx.img_spots {
        let (src, img) = &imgs[idx];
        kprintln!("BROWSER: img '{src}' at ({x}, {y}) {}x{}", img.w, img.h);
    }
    let mut seen: Vec<&str> = Vec::new();
    for l in &ctx.links {
        if !seen.contains(&l.href.as_str()) {
            seen.push(&l.href);
            kprintln!(
                "BROWSER: link '{}' at ({}, {}) {}x{}",
                l.href, l.x, l.y, l.w, l.h
            );
        }
    }
    kprintln!(
        "BROWSER: rendered {path} - {} items, {} links, doc {}x{}",
        ctx.items.len(),
        ctx.links.len(),
        view_w,
        doc_h
    );

    let crate::wm::App::Browser(st) = &mut win.app else { return };
    st.path = path;
    st.page = page;
    st.page_w = view_w;
    st.doc_h = doc_h;
    st.links = ctx.links;
    st.scroll = 0;
    st.page_bg = page_bg;
    paint_view(win);

    if by_click && !M16_DONE.swap(true, Ordering::Relaxed) {
        kprintln!(
            "BROWSER_OK: link click fetched over our TCP from our HTTP server, laid out, painted"
        );
        kprintln!("M16_OK");
    }
    if was_external && !INTERNET_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("BROWSER: rendered external page {path_for_log}");
        kprintln!("INTERNET_OK");
    }
    if is_https(&path_for_log) && !HTTPS_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("BROWSER: rendered https page {path_for_log} (direct TLS 1.3)");
        kprintln!("HTTPS_OK");
    }
}

static INTERNET_DONE: AtomicBool = AtomicBool::new(false);
static HTTPS_DONE: AtomicBool = AtomicBool::new(false);

/// A one-line stand-in page for fetch/parse failures.
fn render_message(win: &mut Window, path: &str, msg: &str) {
    kprintln!("BROWSER: error page for {path}: {msg}");
    let (cw, text) = (win.cw, format!("veil browser: {msg}"));
    let crate::wm::App::Browser(st) = &mut win.app else { return };
    st.path = String::from(path);
    st.page = vec![0xffff_ffff; cw * 64];
    st.page_w = cw;
    st.doc_h = 64;
    st.links = Vec::new();
    st.scroll = 0;
    st.page_bg = 0xffff_ffff;
    let pfb = unsafe { Framebuffer::new(st.page.as_mut_ptr(), cw, 64, cw * 4) };
    pfb.draw_string(8, 8, &text, 0xffa0_2020, None);
    paint_view(win);
}

// --- window plumbing -----------------------------------------------------------

/// Repaint the canvas: visible page rows below a URL bar.
pub fn paint_view(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let bar = {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        let view_h = ch - TOPBAR;
        for row in 0..view_h {
            let sy = st.scroll + row;
            let dst = &mut win.canvas[(TOPBAR + row) * cw..(TOPBAR + row) * cw + cw];
            if sy < st.doc_h && st.page_w == cw {
                dst.copy_from_slice(&st.page[sy * cw..sy * cw + cw]);
            } else {
                dst.fill(st.page_bg);
            }
        }
        let ip = net::local_ip().unwrap_or([0; 4]);
        let pct = if st.doc_h > view_h {
            st.scroll * 100 / (st.doc_h - view_h)
        } else {
            100
        };
        // External pages already carry a full URL; local paths get our IP.
        if st.path.starts_with("http://") || st.path.starts_with("https://") {
            format!("{}  [{pct}%]", st.path)
        } else {
            format!("http://{}{}  [{pct}%]", net::fmt_ip(&ip), st.path)
        }
    };
    // Scrollbar: a 2px gutter on the right edge with a proportional thumb.
    let (doc_h, scroll) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        (st.doc_h, st.scroll)
    };
    let view_h = ch - TOPBAR;
    let fb = win.canvas_fb();
    if doc_h > view_h {
        fb.fill_rect(cw - 2, TOPBAR, 2, view_h, 0xff20_2830); // track
        let thumb_h = (view_h * view_h / doc_h).max(8).min(view_h);
        let thumb_y = TOPBAR + scroll * (view_h - thumb_h) / (doc_h - view_h);
        fb.fill_rect(cw - 2, thumb_y, 2, thumb_h, 0xff70_90b0); // thumb
    }
    fb.fill_rect(0, 0, cw, TOPBAR, BAR_BG);
    // Back button: a `<` in its own 18px zone, then the address bar.
    let has_history = matches!(&win.app, crate::wm::App::Browser(st) if !st.history.is_empty());
    fb.fill_rect(0, 0, 18, TOPBAR, if has_history { 0xff90_a8c0 } else { 0xffb0_b4bc });
    fb.draw_string(5, 2, "<", BAR_TEXT, None);
    fb.draw_string(22, 2, &bar, BAR_TEXT, None);
}

/// Canvas-relative click -> link href, if one was hit.
pub fn link_at(win: &Window, rx: isize, ry: isize) -> Option<String> {
    let crate::wm::App::Browser(st) = &win.app else { return None };
    if ry < TOPBAR as isize {
        return None;
    }
    let (dx, dy) = (rx, ry - TOPBAR as isize + st.scroll as isize);
    st.links
        .iter()
        .find(|l| dx >= l.x && dx < l.x + l.w && dy >= l.y && dy < l.y + l.h)
        .map(|l| l.href.clone())
}

static SCROLL_DONE: AtomicBool = AtomicBool::new(false);

/// Scroll to an absolute offset (clamped to the document), repaint, and emit
/// SCROLL_OK the first time a taller-than-window page moves off its top.
fn scroll_to(win: &mut Window, target: isize) -> bool {
    let view_h = win.ch - TOPBAR;
    let (new, tall) = {
        let crate::wm::App::Browser(st) = &mut win.app else { return false };
        let max = st.doc_h.saturating_sub(view_h);
        let new = target.clamp(0, max as isize) as usize;
        if new == st.scroll {
            return true;
        }
        st.scroll = new;
        (new, st.doc_h > view_h)
    };
    kprintln!("BROWSER: scroll y={new}");
    if tall && new > 0 && !SCROLL_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("SCROLL_OK");
    }
    paint_view(win);
    true
}

fn cur_scroll(win: &Window) -> isize {
    match &win.app {
        crate::wm::App::Browser(st) => st.scroll as isize,
        _ => 0,
    }
}

/// Arrow/page keys scroll: arrows one line, Page keys half a window.
pub fn key(win: &mut Window, code: u16) -> bool {
    const KEY_UP: u16 = 103;
    const KEY_PGUP: u16 = 104;
    const KEY_DOWN: u16 = 108;
    const KEY_PGDN: u16 = 109;
    const KEY_BACKSPACE: u16 = 14;
    if code == KEY_BACKSPACE {
        return back(win); // address bar isn't a text field -> Backspace = back
    }
    let line = 16isize; // one text line
    let half = (win.ch - TOPBAR) as isize / 2;
    let s = cur_scroll(win);
    let target = match code {
        KEY_UP => s - line,
        KEY_DOWN => s + line,
        KEY_PGUP => s - half,
        KEY_PGDN => s + half,
        _ => return false,
    };
    scroll_to(win, target)
}

/// Mouse-wheel scroll. `notches` is the signed REL_WHEEL delta (positive =
/// wheel up / scroll toward the top); three lines per notch.
pub fn wheel(win: &mut Window, notches: i32) -> bool {
    scroll_to(win, cur_scroll(win) - notches as isize * 48)
}
