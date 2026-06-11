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
use crate::{css, font, html, kprintln, net, png, scheduler, timer, tls};

/// A selected bitmap font, or None to use the built-in 8x16 font.
/// A resolved FreeType font: face id + pixel size. Text is rendered
/// anti-aliased via `fb::draw_text` and measured via `glyph_cache::text_width`.
#[derive(Clone, Copy)]
pub struct Font {
    pub id: crate::freetype::FontId,
    pub px: u16,
}

/// Map the CSS-resolved typography to a FreeType face.
fn pick_ftid(fam: font::Family, weight: u16, _italic: bool) -> crate::freetype::FontId {
    use crate::freetype::FontId;
    match fam {
        font::Family::Mono => FontId::Mono,
        font::Family::Cormorant | font::Family::Lora => FontId::Serif,
        _ if weight >= 600 => FontId::UiBold,
        _ => FontId::Ui,
    }
}
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
    // M35 text input: an editable address bar and on-page form fields.
    editing: bool,           // address bar focused for editing
    edit_buf: String,        // address-bar contents while editing
    fields: Vec<InputField>, // <input>/<textarea> on the current page
    focus: Option<usize>,    // index into `fields` of the focused field
    page_text: String,       // all visible text, for Ctrl+A / Ctrl+C
}

/// Copy the page's visible text to the clipboard (Ctrl+A selects all, Ctrl+C
/// copies). Returns the number of bytes copied.
pub fn copy_text(win: &Window) -> usize {
    let crate::wm::App::Browser(st) = &win.app else { return 0 };
    crate::clipboard::set(st.page_text.clone());
    st.page_text.len()
}

/// Paste clipboard text into the focused address bar or input field.
pub fn paste(win: &mut Window) -> bool {
    let text = crate::clipboard::get();
    if text.is_empty() {
        return false;
    }
    let crate::wm::App::Browser(st) = &mut win.app else { return false };
    if st.editing {
        st.edit_buf.push_str(text.trim());
        paint_view(win);
        true
    } else if let Some(f) = st.focus.and_then(|i| st.fields.get_mut(i)) {
        f.value.push_str(text.trim());
        paint_fields(win);
        true
    } else {
        false
    }
}

#[derive(Clone)]
struct InputField {
    x: isize, // document coordinates of the field box
    y: isize,
    w: isize,
    h: isize,
    value: String,
    multiline: bool,
    action: String, // enclosing form's action (for Enter-to-submit)
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
            editing: false,
            edit_buf: String::new(),
            fields: Vec::new(),
            focus: None,
            page_text: String::new(),
        }
    }
}

/// Redraw the on-page input fields into the page buffer (value + focus ring),
/// then repaint the view. Cheaper than a full re-layout for each keystroke.
fn paint_fields(win: &mut Window) {
    let (fields, focus, pw, dh) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        (st.fields.clone(), st.focus, st.page_w, st.doc_h)
    };
    if pw != 0 && dh != 0 {
        if let crate::wm::App::Browser(st) = &mut win.app {
            let pfb = unsafe { Framebuffer::new(st.page.as_mut_ptr(), pw, dh, pw * 4) };
            for (i, f) in fields.iter().enumerate() {
                if f.x < 0 || f.y < 0 {
                    continue;
                }
                let (x, y, w, h) = (f.x as usize, f.y as usize, f.w as usize, f.h as usize);
                pfb.fill_rect(x, y, w, h, 0xff1f_1f1f);
                let border = if focus == Some(i) { 0xff5b_8af0 } else { 0xff4a_5060 };
                pfb.fill_rect(x, y, w, 1, border);
                pfb.fill_rect(x, y + h - 1, w, 1, border);
                pfb.fill_rect(x, y, 1, h, border);
                pfb.fill_rect(x + w - 1, y, 1, h, border);
                let txt = if focus == Some(i) { format!("{}_", f.value) } else { f.value.clone() };
                pfb.draw_string(x + 4, y + 3, &txt, 0xffe8_e8e8, None);
            }
        }
    }
    paint_view(win);
}

/// Click in the chrome (topbar): focus the address bar for editing if the
/// click landed on the URL field (right of the back button). Returns true if
/// the click was consumed by the chrome.
pub fn chrome_click(win: &mut Window, rx: isize, ry: isize) -> bool {
    let crate::wm::App::Browser(st) = &mut win.app else { return false };
    if ry < TOPBAR as isize && rx >= 18 {
        st.editing = true;
        st.focus = None;
        st.edit_buf = String::new(); // focus clears, like select-all + type
        paint_view(win);
        return true;
    }
    false
}

/// A printable character arrived while a text field (address bar or a page
/// input) is focused. Returns true if consumed.
pub fn char_input(win: &mut Window, ch: char) -> bool {
    let crate::wm::App::Browser(st) = &mut win.app else { return false };
    if st.editing {
        st.edit_buf.push(ch);
        paint_view(win);
        true
    } else if let Some(i) = st.focus {
        if let Some(f) = st.fields.get_mut(i) {
            f.value.push(ch);
            // Repaint the page so the typed text shows in the field.
            paint_fields(win);
            return true;
        }
        false
    } else {
        false
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
    // Follow redirects: for a 3xx, hand back the Location as the "body" with a
    // sentinel content-type so navigate() can re-fetch it.
    if (300..400).contains(&status) {
        if let Some(loc) = head.lines().find_map(|l| {
            let (name, val) = l.split_once(':')?;
            name.eq_ignore_ascii_case("location").then(|| val.trim())
        }) {
            kprintln!("BROWSER: GET {path} -> {status} redirect to {loc}");
            return Some((status, String::from("text/redirect"), loc.as_bytes().to_vec()));
        }
    }
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

static DIRECT_HTTP_DONE: AtomicBool = AtomicBool::new(false);

/// Direct HTTP/1.1 fetch of an external `http://` URL over the kernel's own
/// TCP/IP stack — no host proxy. DNS-resolves the host, connects to port 80,
/// sends the GET, and reads until the response is complete (bounded).
fn http_direct(url: &str) -> Option<(u32, String, Vec<u8>)> {
    let rest = url.get(7..)?; // strip "http://"
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(80)),
        None => (hostport, 80u16),
    };
    let ip = net::dns_resolve(host)?;
    kprintln!("BROWSER: direct TCP {host} -> {}.{}.{}.{}:{port}", ip[0], ip[1], ip[2], ip[3]);
    let h = net::tcp_connect(ip, port)?;
    let req =
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: VeilOS\r\nConnection: close\r\n\r\n");
    write_all(h, req.as_bytes());
    let resp = read_http(h, 1 << 20, FETCH_TIMEOUT, FETCH_TIMEOUT * 4);
    net::tcp_close(h);
    let parsed = parse_response(&resp, url)?;
    if parsed.0 == 200 && !DIRECT_HTTP_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("DIRECT_HTTP_OK: fetched {host} over kernel TCP (no host proxy)");
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
    } else if is_external(path) {
        // M35: external http:// goes direct via the kernel TCP stack first.
        if let Some(r) = http_direct(path) {
            return Some(r);
        }
        kprintln!("BROWSER: direct HTTP failed for {path}, falling back to proxy");
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
// The current page's URL, so relative links/srcs/stylesheets on an external
// page resolve against its host (not loopback). None while on the local site.
static mut PAGE_BASE: Option<String> = None;

fn set_page_base(base: Option<String>) {
    unsafe { *core::ptr::addr_of_mut!(PAGE_BASE) = base };
}

fn page_base() -> Option<String> {
    unsafe { (*core::ptr::addr_of!(PAGE_BASE)).clone() }
}

/// Join a possibly-relative URL against an absolute base (scheme://host/path).
fn url_join(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("").trim(); // keep query, drop fragment
    if href.is_empty() {
        return String::from(base);
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let host = rest.split('/').next().unwrap_or(rest);
    if href.starts_with("//") {
        return format!("{scheme}:{href}");
    }
    if href.starts_with('/') {
        return format!("{scheme}://{host}{href}");
    }
    // Relative to the base's directory.
    let path = &rest[host.len()..];
    let dir = match path.rfind('/') {
        Some(i) => &path[..i + 1],
        None => "/",
    };
    format!("{scheme}://{host}{dir}{href}")
}

fn resolve_href(href: &str) -> String {
    let href = href.trim();
    if is_external(href) {
        return String::from(href);
    }
    // On an external page, resolve relative URLs against its host.
    if let Some(base) = page_base() {
        return url_join(&base, href);
    }
    // Local site: strip fragment/query and normalise to an absolute path.
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
    Flex,
}

#[derive(Clone, Copy, PartialEq)]
enum FlexDir {
    Row,
    Column,
}

#[derive(Clone, Copy, PartialEq)]
enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

#[derive(Clone, Copy, PartialEq)]
enum AlignItems {
    Stretch,
    Center,
    Start,
    End,
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
    // Typography (inherits). `font` is the resolved bitmap font (None = 8x16).
    font_fam: font::Family,
    font_weight: u16,
    font_italic: bool,
    font: Font,
    // Flex container properties (meaningful when display == Flex).
    flex_dir: FlexDir,
    flex_wrap: bool,
    justify: Justify,
    align: AlignItems,
    gap: isize,
    // Flex item property (this element as a child of a flex container).
    flex_grow: u32,
    // Hidden-overlay detection (don't inherit). `transparent` = opacity:0,
    // `pointer_none` = pointer-events:none; an element with both is a hidden
    // overlay (e.g. a JS-toggled mobile menu) and is dropped — but opacity:0
    // alone is left visible, since sites also use it for scroll-reveal content
    // that we can't un-hide (no JS).
    transparent: bool,
    pointer_none: bool,
    // Ancestor chain (root..parent) as (tag, raw class attr), for matching
    // descendant selectors like `.nav-links a`. Not a render property.
    anc: Vec<(Option<String>, Option<String>)>,
}

fn root_style() -> Style {
    Style {
        color: 0xff10_1418,
        bg: None,
        scale: 1,
        margin: [0; 4],
        padding: [0; 4],
        display: Display::Block,
        width: None,
        underline: false,
        pre: false,
        font_fam: font::Family::Default,
        font_weight: 400,
        font_italic: false,
        font: Font { id: crate::freetype::FontId::Ui, px: 16 },
        flex_dir: FlexDir::Row,
        flex_wrap: false,
        justify: Justify::Start,
        align: AlignItems::Stretch,
        gap: 0,
        flex_grow: 0,
        transparent: false,
        pointer_none: false,
        anc: Vec::new(),
    }
}

fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    // rgb()/rgba(): integer channels; any alpha is ignored (we render opaque).
    let lower = v.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgb(").or_else(|| lower.strip_prefix("rgba(")) {
        let mut ch = inner.trim_end_matches(')').split(',').map(str::trim);
        let r = ch.next()?.parse::<u32>().ok()?;
        let g = ch.next()?.parse::<u32>().ok()?;
        let b = ch.next()?.parse::<u32>().ok()?;
        return Some(0xff00_0000 | (r & 0xff) << 16 | (g & 0xff) << 8 | (b & 0xff));
    }
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

/// Parse a length to pixels, supporting px, rem/em (1 unit ≈ 16px), pt, and
/// bare numbers. Relative/computed units (%, vw, clamp(), calc()) are unknown.
/// Approximate perceptual luminance (0..255) of an XRGB color.
fn luma(c: u32) -> i32 {
    let (r, g, b) = (((c >> 16) & 0xff) as i32, ((c >> 8) & 0xff) as i32, (c & 0xff) as i32);
    (r * 54 + g * 183 + b * 19) >> 8
}

/// Nudge a text color so it stays legible against `bg`. The site's text colors
/// assume light section backgrounds; on our single (dark) page background, very
/// dark text would vanish. When fg/bg luminance is too close, blend fg ~62%
/// toward white (dark bg) or black (light bg) — keeping a hint of the hue.
/// High-contrast text (the overwhelming common case) is returned unchanged.
fn readable(fg: u32, bg: u32) -> u32 {
    if (luma(fg) - luma(bg)).abs() >= 72 {
        return fg;
    }
    let target: u32 = if luma(bg) < 110 { 0xffff_ffff } else { 0xff00_0000 };
    let mix = |s: u32, d: u32| (s * 3 + d * 5) / 8;
    0xff00_0000
        | mix((fg >> 16) & 0xff, (target >> 16) & 0xff) << 16
        | mix((fg >> 8) & 0xff, (target >> 8) & 0xff) << 8
        | mix(fg & 0xff, target & 0xff)
}

/// Parse a length to pixels, supporting px, rem/em (1 unit ≈ 16px), pt, and
/// bare numbers. Relative/computed units (%, vw, clamp(), calc()) are unknown.
fn parse_px(v: &str) -> Option<isize> {
    let v = v.trim();
    let (num, mul, div) = if let Some(n) = v.strip_suffix("px") {
        (n, 1, 1)
    } else if let Some(n) = v.strip_suffix("rem") {
        (n, 16, 1)
    } else if let Some(n) = v.strip_suffix("em") {
        (n, 16, 1)
    } else if let Some(n) = v.strip_suffix("pt") {
        (n, 4, 3) // 1pt = 4/3 px
    } else {
        (v, 1, 1)
    };
    // Decimal number in tenths (e.g. "2.5" -> 25), to keep integer math.
    let n = num.trim();
    let (int, frac) = n.split_once('.').unwrap_or((n, ""));
    let neg = int.starts_with('-');
    let i: isize = int.trim_start_matches('-').parse().ok().or(int.is_empty().then_some(0))?;
    let f = frac.bytes().next().map_or(0, |b| (b.wrapping_sub(b'0')).min(9) as isize);
    let tenths = i * 10 + f;
    Some((if neg { -tenths } else { tenths }) * mul / div / 10)
}

// CSS custom properties for the current document. Rendering is single-threaded
// on the desktop task, so a global set-before-layout map is enough — no need to
// thread it through every resolve()/apply_decl() call.
static mut CSS_VARS: Option<Vec<(String, String)>> = None;
static CSS_VAR_DONE: AtomicBool = AtomicBool::new(false);

fn set_css_vars(v: Vec<(String, String)>) {
    unsafe { *core::ptr::addr_of_mut!(CSS_VARS) = Some(v) };
}

fn lookup_var(name: &str) -> Option<String> {
    unsafe {
        (*core::ptr::addr_of!(CSS_VARS))
            .as_ref()?
            .iter()
            .rev() // later declarations win
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

/// Resolve `var(--name)` / `var(--name, fallback)` references in a value,
/// iterating a few times so variables that reference other variables resolve.
fn substitute_vars(val: &str) -> String {
    let mut cur = String::from(val);
    for _ in 0..4 {
        if !cur.contains("var(") {
            break;
        }
        let mut out = String::new();
        let mut rest = cur.as_str();
        while let Some(i) = rest.find("var(") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 4..];
            let Some(close) = after.find(')') else {
                out.push_str(&rest[i..]);
                rest = "";
                break;
            };
            let inner = &after[..close];
            let (name, fallback) = match inner.split_once(',') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (inner.trim(), None),
            };
            let resolved = lookup_var(name)
                .or_else(|| fallback.map(String::from))
                .unwrap_or_default();
            if !CSS_VAR_DONE.swap(true, Ordering::Relaxed) {
                kprintln!("CSS_VAR_OK");
            }
            out.push_str(&resolved);
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        cur = out;
    }
    cur
}

fn apply_decl(s: &mut Style, prop: &str, val: &str) {
    // Custom properties are stored globally, not applied to the element style.
    if prop.starts_with("--") {
        return;
    }
    let substituted;
    let val = if val.contains("var(") {
        substituted = substitute_vars(val);
        substituted.as_str()
    } else {
        val
    };
    match prop {
        "color" => {
            if let Some(c) = parse_color(val) {
                s.color = c;
            }
        }
        "background-color" => s.bg = parse_color(val),
        "background" => {
            // Shorthand: take the first colour-like token (after var() expansion).
            for tok in val.split([' ', ',']).filter(|t| !t.is_empty()) {
                if let Some(c) = parse_color(tok) {
                    s.bg = Some(c);
                    break;
                }
            }
        }
        "font-size" => {
            if let Some(px) = parse_px(val) {
                s.scale = (((px + 8) / 16).max(1) as usize).min(4);
            }
        }
        "font-family" => s.font_fam = font::match_family(val),
        "font-weight" => {
            s.font_weight = match val.trim() {
                "bold" | "bolder" => 700,
                "normal" | "lighter" => 400,
                n => n.parse().unwrap_or(s.font_weight),
            };
        }
        "font-style" => s.font_italic = val.trim().starts_with("italic") || val.trim().starts_with("oblique"),
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
                "inline" | "inline-block" => Display::Inline,
                "block" => Display::Block,
                "flex" | "inline-flex" => Display::Flex,
                _ => s.display,
            }
        }
        "flex-direction" => {
            s.flex_dir = if val.trim().starts_with("column") { FlexDir::Column } else { FlexDir::Row };
        }
        "flex-wrap" => s.flex_wrap = val.trim().starts_with("wrap"),
        "justify-content" => {
            s.justify = match val.trim() {
                "center" => Justify::Center,
                "flex-end" | "end" | "right" => Justify::End,
                "space-between" => Justify::SpaceBetween,
                "space-around" | "space-evenly" => Justify::SpaceAround,
                _ => Justify::Start,
            }
        }
        "align-items" => {
            s.align = match val.trim() {
                "center" => AlignItems::Center,
                "flex-end" | "end" => AlignItems::End,
                "flex-start" | "start" => AlignItems::Start,
                _ => AlignItems::Stretch,
            }
        }
        "gap" | "grid-gap" => s.gap = parse_px(val).unwrap_or(s.gap),
        "flex" => {
            // `flex: <grow>` (also accept the shorthand's first number).
            let first = val.split_whitespace().next().unwrap_or("0");
            s.flex_grow = first.parse::<u32>().unwrap_or(if first == "none" { 0 } else { 1 });
        }
        "flex-grow" => s.flex_grow = val.trim().parse().unwrap_or(s.flex_grow),
        "opacity" => {
            // Note transparency; whether it hides depends on pointer-events too
            // (decided after the whole cascade, in `resolve`).
            let v = val.trim();
            s.transparent = !v.is_empty() && v.bytes().all(|b| b == b'0' || b == b'.');
        }
        "pointer-events" => s.pointer_none = val.trim() == "none",
        "visibility" => {
            if matches!(val.trim(), "hidden" | "collapse") {
                s.display = Display::None;
            }
        }
        "text-decoration" | "text-decoration-line" => {
            if val.contains("none") {
                s.underline = false;
            } else if val.contains("underline") {
                s.underline = true;
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
        // Typography inherits.
        font_fam: inherited.font_fam,
        font_weight: inherited.font_weight,
        font_italic: inherited.font_italic,
        font: inherited.font,
        // Flex properties don't inherit — reset to defaults each element.
        flex_dir: FlexDir::Row,
        flex_wrap: false,
        justify: Justify::Start,
        align: AlignItems::Stretch,
        gap: 0,
        flex_grow: 0,
        transparent: false,
        pointer_none: false,
        anc: Vec::new(),
    };
    match tag {
        "html" => s.display = Display::Block,
        "body" => {
            s.display = Display::Block;
            s.margin = [8; 4];
        }
        "div" | "section" | "nav" | "header" | "footer" | "main" | "article" | "aside"
        | "figure" | "figcaption" | "blockquote" => s.display = Display::Block,
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
        "pre" | "code" => {
            s.display = Display::Block;
            s.margin[0] = 8;
            s.margin[2] = 8;
            s.pre = true;
            s.font_fam = font::Family::Mono; // <pre>/<code> default to monospace
        }
        "a" => {
            s.color = 0xff20_50c0;
            s.underline = true;
        }
        "table" | "tr" | "td" | "th" => s.display = Display::Block,
        "head" | "title" | "script" | "style" | "meta" | "link" => s.display = Display::None,
        _ => {} // span, img, unknown: inline containers
    }
    // Ancestor chain (root..parent) as borrowed (tag, class) pairs, for
    // descendant-selector matching.
    let anc: Vec<(&str, Option<&str>)> = inherited
        .anc
        .iter()
        .map(|(t, c)| (t.as_deref().unwrap_or(""), c.as_deref()))
        .collect();
    // Apply every matching rule in ascending specificity. A stable sort keeps
    // source order within a rank, so a later equal-rank rule still wins (the
    // cascade). Tag UA defaults above are rank -∞, applied first.
    let mut matched: Vec<(u32, usize)> = Vec::new();
    for (i, r) in sheet.iter().enumerate() {
        if r.matches(tag, class, &anc) {
            matched.push((r.rank(), i));
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, i) in matched {
        for (p, v) in &sheet[i].decls {
            apply_decl(&mut s, p, v);
        }
    }
    // A fully-transparent, non-interactive element is a hidden overlay (a
    // JS-toggled mobile menu, a dialog backdrop): drop it. opacity:0 on its own
    // is scroll-reveal content — leave it visible since no JS will reveal it.
    if s.transparent && s.pointer_none {
        s.display = Display::None;
    }
    // Record this node so its descendants can match `ancestor key` selectors.
    s.anc = inherited.anc.clone();
    s.anc.push((Some(String::from(tag)), class.map(String::from)));
    // Resolve the FreeType face + pixel size from the (inherited) typography.
    s.font = Font {
        id: pick_ftid(s.font_fam, s.font_weight, s.font_italic),
        px: ((s.scale * 16) as u16).max(13),
    };
    s
}

// --- layout ---------------------------------------------------------------------

enum Item {
    Rect { x: isize, y: isize, w: isize, h: isize, color: u32 },
    Text { x: isize, y: isize, s: String, color: u32, scale: usize, font: Font },
    Image { x: isize, y: isize, idx: usize },
}

enum Frag {
    Word { s: String, color: u32, scale: usize, link: Option<String>, underline: bool, font: Font },
    Space { scale: usize, font: Font },
    Img { idx: usize, w: isize, h: isize },
    Input { value: String, w: isize, h: isize, multiline: bool },
    Br,
}

/// Pixel advance width of a text run in its FreeType face.
fn text_w(s: &str, _scale: usize, f: Font) -> isize {
    crate::glyph_cache::text_width(s, f.id, f.px) as isize
}

/// Line height for the run (FreeType px size, ~1.25x for leading).
fn text_h(_scale: usize, f: Font) -> isize {
    (f.px as isize * 5) / 4
}

fn frag_w(f: &Frag) -> isize {
    match f {
        Frag::Word { s, scale, font, .. } => text_w(s, *scale, *font),
        Frag::Space { scale, font } => text_w(" ", *scale, *font),
        Frag::Img { w, .. } | Frag::Input { w, .. } => *w,
        Frag::Br => 0,
    }
}

fn frag_h(f: &Frag) -> isize {
    match f {
        Frag::Word { scale, font, .. } | Frag::Space { scale, font } => text_h(*scale, *font),
        Frag::Img { h, .. } | Frag::Input { h, .. } => *h,
        Frag::Br => 16,
    }
}

struct Ctx<'a> {
    sheet: &'a [css::Rule],
    imgs: &'a [(String, png::Image)],
    items: Vec<Item>,
    links: Vec<LinkBox>,
    img_spots: Vec<(usize, isize, isize)>, // (imgs idx, x, y) for the proof log
    fields: Vec<InputField>,               // on-page form fields, with positions
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
                font: style.font,
            });
        }
    };
    for c in t.chars() {
        if c.is_ascii_whitespace() {
            flush(&mut word, buf);
            if !matches!(buf.last(), Some(Frag::Space { .. }) | None) {
                buf.push(Frag::Space { scale: style.scale, font: style.font });
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
                "input" => {
                    // Render text-like inputs as editable boxes; skip the rest.
                    let ty = node.attr("type").unwrap_or("text").to_ascii_lowercase();
                    if matches!(ty.as_str(), "text" | "search" | "email" | "url" | "password" | "tel" | "number" | "") {
                        let value = node.attr("value").unwrap_or("").into();
                        let chars = node.attr("size").and_then(|s| s.parse::<isize>().ok()).unwrap_or(20);
                        buf.push(Frag::Input { value, w: (chars * 8 + 8).clamp(80, 360), h: 20, multiline: false });
                    }
                }
                "textarea" => {
                    let mut value = String::new();
                    node.text(&mut value);
                    buf.push(Frag::Input { value: value.trim().into(), w: 280, h: 64, multiline: true });
                }
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
            Frag::Word { s, color, scale, link, underline, font } => {
                let fw = text_w(&s, scale, font);
                let fh = text_h(scale, font);
                if underline {
                    ctx.items.push(Item::Rect {
                        x: x + dx,
                        y: fy + fh - scale as isize,
                        w: fw,
                        h: scale as isize,
                        color,
                    });
                }
                if let Some(href) = link {
                    ctx.links.push(LinkBox { x: x + dx, y: fy, w: fw, h: fh, href });
                }
                ctx.items.push(Item::Text { x: x + dx, y: fy, s, color, scale, font });
            }
            Frag::Img { idx, .. } => {
                ctx.img_spots.push((idx, x + dx, fy));
                ctx.items.push(Item::Image { x: x + dx, y: fy, idx });
            }
            Frag::Input { value, w: fw, h: fh, multiline } => {
                let (bx, by) = (x + dx, fy);
                // Field surface + a 1px muted border (focus ring drawn later).
                ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff1f1f1f });
                for (rx, ry, rw, rh) in
                    [(bx, by, fw, 1), (bx, by + fh - 1, fw, 1), (bx, by, 1, fh), (bx + fw - 1, by, 1, fh)]
                {
                    ctx.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: 0xff4a5060 });
                }
                ctx.items.push(Item::Text {
                    x: bx + 4,
                    y: by + 3,
                    s: value.clone(),
                    color: 0xffe8e8e8,
                    scale: 1,
                    font: Font { id: crate::freetype::FontId::Ui, px: 16 },
                });
                ctx.fields.push(InputField { x: bx, y: by, w: fw, h: fh, value, multiline, action: String::new() });
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
            font: Font { id: crate::freetype::FontId::Ui, px: 16 },
        });
    }

    if style.pre {
        // Verbatim: no wrap, no whitespace collapsing.
        let mut text = String::new();
        node.text(&mut text);
        let lh = text_h(style.scale, style.font);
        for line in text.trim_matches('\n').lines() {
            ctx.items.push(Item::Text {
                x: cx,
                y: cy,
                s: String::from(line),
                color: style.color,
                scale: style.scale,
                font: style.font,
            });
            cy += lh;
        }
    } else if style.display == Display::Flex {
        cy = layout_flex(ctx, node, &style, cx, cw, cy);
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
            && matches!(cstyle.display, Display::Block | Display::Flex);
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

// --- flexbox ----------------------------------------------------------------

static FLEX_DONE: AtomicBool = AtomicBool::new(false);

fn item_right(it: &Item, ctx: &Ctx) -> isize {
    match it {
        Item::Text { x, s, scale, font, .. } => x + text_w(s, *scale, *font),
        Item::Image { x, idx, .. } => x + ctx.imgs[*idx].1.w as isize,
        Item::Rect { x, w, .. } => x + w,
    }
}

/// Lay a node out in isolation at `avail_w` and return its positioned items
/// plus its measured content width (text/image extent — background/border
/// rects that fill `avail_w` are ignored) and total height.
fn measure_item(
    ctx: &Ctx, node: &html::Node, parent: &Style, avail_w: isize,
) -> (Vec<Item>, Vec<LinkBox>, Vec<(usize, isize, isize)>, isize, isize) {
    let mut tmp = Ctx {
        sheet: ctx.sheet,
        imgs: ctx.imgs,
        items: Vec::new(),
        links: Vec::new(),
        img_spots: Vec::new(),
        fields: Vec::new(),
    };
    let cstyle = resolve(ctx.sheet, node, parent);
    let bottom = if cstyle.display == Display::Inline {
        let mut buf = Vec::new();
        collect_inline(&mut tmp, node, &cstyle, &None, &mut buf);
        flush_inline(&mut tmp, &mut buf, 0, avail_w, 0)
    } else {
        layout_block(&mut tmp, node, parent, 0, avail_w, 0, None)
    };
    // Content width from text/image items only (ignore full-width bg rects).
    let mut content_w = 0;
    for it in &tmp.items {
        if !matches!(it, Item::Rect { .. }) {
            content_w = content_w.max(item_right(it, ctx));
        }
    }
    if content_w == 0 {
        // No text/image (e.g. an empty coloured box): fall back to bg width.
        for it in &tmp.items {
            content_w = content_w.max(item_right(it, ctx));
        }
    }
    (tmp.items, tmp.links, tmp.img_spots, content_w.min(avail_w), bottom)
}

/// Shift a measured item set by (dx, dy) and append it to `dst`.
fn place(
    dst: &mut Ctx, mut items: Vec<Item>, mut links: Vec<LinkBox>,
    mut spots: Vec<(usize, isize, isize)>, dx: isize, dy: isize,
) {
    for it in &mut items {
        match it {
            Item::Rect { x, y, .. } | Item::Text { x, y, .. } | Item::Image { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
        }
    }
    for l in &mut links {
        l.x += dx;
        l.y += dy;
    }
    for s in &mut spots {
        s.1 += dx;
        s.2 += dy;
    }
    dst.items.extend(items);
    dst.links.extend(links);
    dst.img_spots.extend(spots);
}

fn align_offset(a: AlignItems, line: isize, item: isize) -> isize {
    match a {
        AlignItems::Start | AlignItems::Stretch => 0,
        AlignItems::Center => (line - item) / 2,
        AlignItems::End => line - item,
    }
}

/// (leading offset before the first item, extra space added after each item)
/// for `justify-content` given the free main-axis space.
fn justify_layout(j: Justify, free: isize, n: isize) -> (isize, isize) {
    if free <= 0 || n <= 0 {
        return (0, 0);
    }
    match j {
        Justify::Start => (0, 0),
        Justify::End => (free, 0),
        Justify::Center => (free / 2, 0),
        Justify::SpaceBetween => (0, if n > 1 { free / (n - 1) } else { 0 }),
        Justify::SpaceAround => {
            let s = free / n;
            (s / 2, s)
        }
    }
}

struct FlexItem<'a> {
    node: &'a html::Node,
    main: isize,
    cross: isize,
    grow: u32,
    width: Option<isize>,
}

fn layout_flex(ctx: &mut Ctx, node: &html::Node, style: &Style, x: isize, w: isize, y: isize) -> isize {
    if !FLEX_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("FLEX_OK");
    }
    let dir_row = style.flex_dir == FlexDir::Row;
    let gap = style.gap;
    // Flex items: element children (bare whitespace text between them is ignored).
    let mut fis: Vec<FlexItem> = Vec::new();
    for c in node.children() {
        if !matches!(c, html::Node::Element { .. }) {
            continue;
        }
        let cs = resolve(ctx.sheet, c, style);
        if cs.display == Display::None {
            continue;
        }
        let (_, _, _, cw, ch) = measure_item(ctx, c, style, w);
        let nat_w = cs.width.unwrap_or(cw).max(1);
        let (main, cross) = if dir_row { (nat_w, ch) } else { (ch, nat_w) };
        fis.push(FlexItem { node: c, main, cross, grow: cs.flex_grow, width: cs.width });
    }
    if fis.is_empty() {
        return y;
    }

    if !dir_row {
        // Column: stack vertically with gaps; align-items on the horizontal axis.
        let mut cy = y;
        for (i, fi) in fis.iter().enumerate() {
            if i > 0 {
                cy += gap;
            }
            let item_w = if style.align == AlignItems::Stretch {
                w
            } else {
                fi.cross.min(w)
            };
            let off = align_offset(style.align, w, item_w);
            let (items, links, spots, _, _) = measure_item(ctx, fi.node, style, item_w);
            place(ctx, items, links, spots, x + off, cy);
            cy += fi.main;
        }
        return cy;
    }

    // Row: break into wrap lines, then justify + align each line.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_main = 0isize;
    for (i, fi) in fis.iter().enumerate() {
        let add = fi.main + if cur.is_empty() { 0 } else { gap };
        if style.flex_wrap && !cur.is_empty() && cur_main + add > w {
            lines.push(core::mem::take(&mut cur));
            cur_main = 0;
        }
        cur_main += fi.main + if cur.is_empty() { 0 } else { gap };
        cur.push(i);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }

    let mut cy = y;
    for line in &lines {
        let natural: isize = line.iter().map(|&i| fis[i].main).sum();
        let gaps = gap * (line.len() as isize - 1).max(0);
        let total_grow: u32 = line.iter().map(|&i| fis[i].grow).sum();
        let free = (w - natural - gaps).max(0);
        let line_h = line.iter().map(|&i| fis[i].cross).max().unwrap_or(0);
        let (lead, between) = if total_grow > 0 {
            (0, 0) // grow consumes the free space instead of justify spacing
        } else {
            justify_layout(style.justify, free, line.len() as isize)
        };
        let mut px = x + lead;
        for (k, &i) in line.iter().enumerate() {
            if k > 0 {
                px += gap + between;
            }
            let extra = if total_grow > 0 {
                free * fis[i].grow as isize / total_grow as isize
            } else {
                0
            };
            let item_w = fis[i].width.unwrap_or(fis[i].main) + extra;
            let cross_off = align_offset(style.align, line_h, fis[i].cross);
            let (items, links, spots, _, _) = measure_item(ctx, fis[i].node, style, item_w.max(1));
            place(ctx, items, links, spots, px, cy + cross_off);
            px += item_w;
        }
        cy += line_h + gap;
    }
    (cy - gap).max(y)
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
    // Set the base so this page's relative stylesheets/images/links resolve
    // against its own host (external) or stay loopback-relative (local).
    set_page_base(if was_external { Some(path.clone()) } else { None });
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
    // Fetch, following up to a few 3xx redirects (Location may be relative).
    let mut path = path;
    let (status, ctype, body) = {
        let mut result = None;
        for _ in 0..6 {
            let Some((s, c, b)) = http_get(&path) else { break };
            if c == "text/redirect" {
                let loc = String::from_utf8_lossy(&b);
                let loc = loc.trim();
                let next = if is_external(loc) { String::from(loc) } else { url_join(&path, loc) };
                kprintln!("BROWSER: following redirect -> {next}");
                set_page_base(if is_external(&next) { Some(next.clone()) } else { None });
                path = next;
                continue;
            }
            result = Some((s, c, b));
            break;
        }
        match result {
            Some(r) => r,
            None => {
                render_message(win, &path, "fetch failed: no response from server");
                return;
            }
        }
    };
    if !ctype.to_ascii_lowercase().contains("text/html") {
        render_message(win, &path, &format!("not html: {ctype} ({status})"));
        return;
    }
    let doc = html::parse(&String::from_utf8_lossy(&body));

    // Stylesheets: linked (<link rel="stylesheet">) and inline (<style>). Both
    // contribute rules and CSS custom properties. `all_css` accumulates the raw
    // text so :root variables (which css::parse skips as a selector) are still
    // collected.
    let mut sheet: Vec<css::Rule> = Vec::new();
    let mut all_css = String::new();
    let mut link_nodes = Vec::new();
    doc.find_all("link", &mut link_nodes);
    for l in link_nodes {
        let rel = l.attr("rel").unwrap_or_default();
        if !rel.eq_ignore_ascii_case("stylesheet") {
            continue;
        }
        let Some(href) = l.attr("href") else { continue };
        if let Some((200, _, css_body)) = http_get(&resolve_href(href)) {
            let css = String::from_utf8_lossy(&css_body).into_owned();
            sheet.extend(css::parse(&css));
            all_css.push_str(&css);
            all_css.push('\n');
        }
    }
    let mut style_nodes = Vec::new();
    doc.find_all("style", &mut style_nodes);
    for st in style_nodes {
        let mut css = String::new();
        st.text(&mut css);
        sheet.extend(css::parse(&css));
        all_css.push_str(&css);
        all_css.push('\n');
    }
    set_css_vars(css::collect_vars(&all_css));

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
            match png::decode_any(&data) {
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
    let root = root_style();
    let body_style = resolve(&sheet, body_node, &root);
    let page_bg = body_style.bg.unwrap_or(0xffff_ffff);
    let mut ctx = Ctx {
        sheet: &sheet,
        imgs: &imgs,
        items: Vec::new(),
        links: Vec::new(),
        img_spots: Vec::new(),
        fields: Vec::new(),
    };
    let end_y = layout_block(&mut ctx, body_node, &root, 0, view_w as isize, 0, None);
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
            Item::Text { x, y, s, color, scale, font } => {
                if *y >= 0 {
                    // Keep text legible against the page: the site's colors
                    // assume light section backgrounds we don't paint.
                    let color = readable(*color, page_bg);
                    let _ = scale;
                    pfb.draw_text((*x).max(0) as usize, *y as usize, s, font.id, font.px, color);
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
    st.fields = ctx.fields;
    let mut text = String::new();
    for it in &ctx.items {
        if let Item::Text { s, .. } = it {
            text.push_str(s);
            text.push(' ');
        }
    }
    st.page_text = text;
    st.focus = None;
    st.scroll = 0;
    st.page_bg = page_bg;
    let nfields = st.fields.len();
    for f in &st.fields {
        kprintln!("BROWSER: field 'input' at ({}, {}) {}x{}", f.x, f.y, f.w, f.h);
    }
    paint_view(win);
    if nfields > 0 {
        kprintln!("BROWSER: {nfields} form field(s) on page");
    }

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
        // While editing, show the edit buffer with a block cursor.
        if st.editing {
            format!("{}_", st.edit_buf)
        } else if st.path.starts_with("http://") || st.path.starts_with("https://") {
            // External pages already carry a full URL; local paths get our IP.
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
        // Thin (4px) rounded scrollbar: #444 track, #888 thumb.
        fb.fill_round_rect(cw - 5, TOPBAR + 1, 4, view_h - 2, 2, 0xff44_4444);
        let thumb_h = (view_h * view_h / doc_h).max(16).min(view_h);
        let thumb_y = TOPBAR + scroll * (view_h - thumb_h) / (doc_h - view_h);
        fb.fill_round_rect(cw - 5, thumb_y, 4, thumb_h, 2, 0xff88_8888);
    }
    fb.fill_rect(0, 0, cw, TOPBAR, BAR_BG);
    // Back button: a `<` in its own 18px zone, then the address bar.
    let has_history = matches!(&win.app, crate::wm::App::Browser(st) if !st.history.is_empty());
    fb.fill_rect(0, 0, 18, TOPBAR, if has_history { 0xff90_a8c0 } else { 0xffb0_b4bc });
    fb.draw_string(5, 2, "<", BAR_TEXT, None);
    fb.draw_string(22, 2, &bar, BAR_TEXT, None);
}

/// Canvas-relative click: focus an on-page input field if one was hit.
/// Returns true if a field took focus.
pub fn focus_field(win: &mut Window, rx: isize, ry: isize) -> bool {
    let crate::wm::App::Browser(st) = &mut win.app else { return false };
    if ry < TOPBAR as isize {
        return false;
    }
    let (dx, dy) = (rx, ry - TOPBAR as isize + st.scroll as isize);
    if let Some(i) = st
        .fields
        .iter()
        .position(|f| dx >= f.x && dx < f.x + f.w && dy >= f.y && dy < f.y + f.h)
    {
        st.focus = Some(i);
        st.editing = false;
        paint_fields(win);
        return true;
    }
    false
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
    const KEY_ENTER: u16 = 28;
    const KEY_ESC: u16 = 1;

    // Address-bar editing intercepts the keyboard.
    let editing = matches!(&win.app, crate::wm::App::Browser(st) if st.editing);
    if editing {
        match code {
            KEY_ENTER => {
                let url = if let crate::wm::App::Browser(st) = &mut win.app {
                    st.editing = false;
                    core::mem::take(&mut st.edit_buf)
                } else {
                    return true;
                };
                kprintln!("BROWSER: address bar -> {url}");
                navigate(win, url.trim(), true);
                return true;
            }
            KEY_ESC => {
                if let crate::wm::App::Browser(st) = &mut win.app {
                    st.editing = false;
                }
                paint_view(win);
                return true;
            }
            KEY_BACKSPACE => {
                if let crate::wm::App::Browser(st) = &mut win.app {
                    st.edit_buf.pop();
                }
                paint_view(win);
                return true;
            }
            // Let character-producing keys fall through to char_input(); other
            // non-char keys (arrows etc.) are simply ignored while editing.
            _ => return false,
        }
    }

    // A focused on-page input field takes Enter (submit) and Backspace (delete).
    let focused = matches!(&win.app, crate::wm::App::Browser(st) if st.focus.is_some());
    if focused {
        match code {
            KEY_BACKSPACE => {
                if let crate::wm::App::Browser(st) = &mut win.app {
                    if let Some(f) = st.focus.and_then(|i| st.fields.get_mut(i)) {
                        f.value.pop();
                    }
                }
                paint_fields(win);
                return true;
            }
            KEY_ENTER => {
                let action = if let crate::wm::App::Browser(st) = &mut win.app {
                    let a = st.focus.and_then(|i| st.fields.get(i)).map(|f| f.action.clone());
                    st.focus = None;
                    a
                } else {
                    None
                };
                if let Some(a) = action.filter(|a| !a.is_empty()) {
                    kprintln!("BROWSER: form submit -> {a}");
                    navigate(win, &a, true);
                }
                paint_fields(win);
                return true;
            }
            KEY_ESC => {
                if let crate::wm::App::Browser(st) = &mut win.app {
                    st.focus = None;
                }
                paint_fields(win);
                return true;
            }
            _ => {}
        }
    }

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
