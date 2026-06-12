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

/// Map the CSS-resolved typography to a FreeType face. Prefers an actual web
/// font (registered from the page's @font-face rules) over the bundled
/// fallbacks, so e.g. Cormorant Garamond renders in its real typeface.
fn pick_ftid(fam: font::Family, weight: u16, italic: bool) -> crate::freetype::FontId {
    use crate::freetype::{find_web_font, FontId};
    let web_name = match fam {
        font::Family::Cormorant => Some("Cormorant Garamond"),
        font::Family::Lora => Some("Lora"),
        font::Family::Barlow => Some("Barlow Condensed"),
        _ => None,
    };
    if let Some(name) = web_name {
        if let Some(i) = find_web_font(name, weight, italic) {
            return FontId::Web(i);
        }
    }
    match fam {
        font::Family::Mono => FontId::Mono,
        font::Family::Cormorant | font::Family::Lora => FontId::Serif,
        _ if weight >= 600 => FontId::UiBold,
        _ => FontId::Ui,
    }
}
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

/// Current page zoom (percent), threaded into layout's font-size computation
/// without plumbing it through every `resolve` call. Set at navigate time.
static ZOOM: AtomicU16 = AtomicU16::new(100);

pub const TOPBAR: usize = 20; // address-bar row height
pub const TABBAR_H: usize = 22; // tab-strip row height (above the address bar)
pub const CHROME: usize = TABBAR_H + TOPBAR; // total chrome; page content starts here
const MAX_DOC_H: usize = 16000; // logical doc height cap (band rasterizer, not the buffer)
const BAND_H: usize = 2000; // tallest rasterized band; repainted on scroll
const DEFAULT_IMG_W: isize = 120; // placeholder box for a deferred, attr-less <img>
const DEFAULT_IMG_H: isize = 90;
const FETCH_TIMEOUT: u64 = 500; // 10 s at the 50 Hz tick
const BAR_BG: u32 = 0xffc8_ccd4;
const BAR_TEXT: u32 = 0xff20_2830;

static M16_DONE: AtomicBool = AtomicBool::new(false);

pub struct BrowserState {
    pub path: String,
    // The document is kept as a retained display list (`items` + `imgs`) and
    // only a *band* of it is rasterized at a time into `page` (page_w * band_h),
    // repositioned + repainted as the user scrolls. This caps the buffer at a
    // few MB even for 10000px+ JS pages, so the whole page stays scrollable.
    page: Vec<u32>, // page_w * band_h, the rasterized band
    page_w: usize,
    doc_h: usize,     // full logical document height (the scroll range)
    band_top: usize,  // document y at which the band buffer currently starts
    items: Vec<Item>, // retained display list, painted per band
    imgs: Vec<Option<png::Image>>, // decoded images by slot; None = not fetched yet
    img_src: Vec<String>,          // source URL per slot, for lazy (on-scroll) loading
    links: Vec<LinkBox>,
    scroll: usize,
    page_bg: u32,
    // M39 tabs: each tab carries its own path + scroll + back/forward stacks +
    // title. The active tab's *rendered* state lives in the fields above; `tabs`
    // is the lightweight per-tab metadata (re-rendered on switch).
    tabs: Vec<Tab>,
    active: usize,
    zoom: u16, // page zoom, percent (50..250)
    img_cache: Vec<(String, png::Image)>, // decoded images by URL, LRU, cap 10
    // M35 text input: an editable address bar and on-page form fields.
    editing: bool,           // address bar focused for editing
    edit_buf: String,        // address-bar contents while editing
    fields: Vec<InputField>, // <input>/<textarea>/<select> on the current page
    forms: Vec<Form>,        // forms on the current page (method + action)
    focus: Option<usize>,    // index into `fields` of the focused field
    page_text: String,       // all visible text, for Ctrl+A / Ctrl+C
    // M36 find-in-page (Ctrl+F).
    text_runs: Vec<(isize, isize, isize, String)>, // (x, y, w, lowercased) in page coords
    find_open: bool,
    find_query: String,
    find_matches: Vec<usize>, // indices into text_runs that match
    find_idx: usize,
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

/// Kind of form control.
#[derive(Clone, Copy, PartialEq)]
enum InputKind {
    Text,
    Password,
    Hidden,
    Checkbox,
    Radio,
    Submit,
    Textarea,
    Select,
}

#[derive(Clone)]
struct InputField {
    x: isize, // document coordinates of the field box
    y: isize,
    w: isize,
    h: isize,
    name: String,
    value: String,
    kind: InputKind,
    checked: bool,
    multiline: bool,
    form: usize, // index into BrowserState.forms (usize::MAX = no enclosing form)
    options: Vec<String>, // for <select>
}

/// An HTML form's submission target.
#[derive(Clone)]
struct Form {
    method: String, // "GET" or "POST"
    action: String,
}

struct LinkBox {
    x: isize,
    y: isize,
    w: isize,
    h: isize,
    href: String,
}

/// Per-tab navigable state. The active tab's rendered page lives in the
/// BrowserState fields; this is the lightweight metadata for every tab.
#[derive(Clone)]
struct Tab {
    path: String,
    scroll: usize,
    back: Vec<String>, // visited paths, newest last
    fwd: Vec<String>,  // popped-via-back paths, for forward
    title: String,
}

impl Tab {
    fn new(path: &str) -> Tab {
        Tab { path: String::from(path), scroll: 0, back: Vec::new(), fwd: Vec::new(), title: String::from(path) }
    }
}

impl BrowserState {
    pub fn new() -> BrowserState {
        BrowserState {
            path: String::from("/"),
            page: vec![0xffff_ffff; 1],
            page_w: 1,
            doc_h: 1,
            band_top: 0,
            items: Vec::new(),
            imgs: Vec::new(),
            img_src: Vec::new(),
            links: Vec::new(),
            scroll: 0,
            page_bg: 0xffff_ffff,
            tabs: alloc::vec![Tab::new("/")],
            active: 0,
            zoom: 100,
            img_cache: Vec::new(),
            editing: false,
            edit_buf: String::new(),
            fields: Vec::new(),
            forms: Vec::new(),
            focus: None,
            page_text: String::new(),
            text_runs: Vec::new(),
            find_open: false,
            find_query: String::new(),
            find_matches: Vec::new(),
            find_idx: 0,
        }
    }
}

/// Draw the live state of the on-page form controls (value + focus ring +
/// checkbox/radio fill) into the band buffer `pfb`, with document row
/// `band_top` mapped to buffer row 0. Controls outside the band are skipped.
fn paint_field_overlays(pfb: &Framebuffer, fields: &[InputField], focus: Option<usize>, band_top: isize, bh: isize) {
    for (i, f) in fields.iter().enumerate() {
        if f.x < 0 || f.y < 0 || f.kind == InputKind::Hidden {
            continue;
        }
        let yy = f.y - band_top;
        if yy < 0 || yy + f.h > bh {
            continue; // not (fully) in the rasterized band
        }
        let (x, y, w, h) = (f.x as usize, yy as usize, f.w as usize, f.h as usize);
        let focused = focus == Some(i);
        let border = if focused { 0xff5b_8af0 } else { 0xff4a_5060 };
        let frame = |pfb: &Framebuffer, bc: u32| {
            pfb.fill_rect(x, y, w, 1, bc);
            pfb.fill_rect(x, y + h - 1, w, 1, bc);
            pfb.fill_rect(x, y, 1, h, bc);
            pfb.fill_rect(x + w - 1, y, 1, h, bc);
        };
        match f.kind {
            InputKind::Checkbox => {
                pfb.fill_rect(x, y, w, h, 0xff2a_2a2a);
                frame(pfb, 0xff8a_90a0);
                if f.checked && w > 6 && h > 6 {
                    pfb.fill_rect(x + 3, y + 3, w - 6, h - 6, 0xff5b_8af0);
                }
            }
            InputKind::Radio => {
                pfb.fill_rect(x, y, w, h, 0xff2a_2a2a);
                frame(pfb, 0xff8a_90a0);
                if f.checked && w > 8 && h > 8 {
                    pfb.fill_rect(x + 4, y + 4, w - 8, h - 8, 0xff5b_8af0);
                }
            }
            InputKind::Submit => {
                pfb.fill_rect(x, y, w, h, if focused { 0xff4a_82c5 } else { 0xff3a_6ea5 });
                pfb.draw_string(x + 10, y + 5, &f.value, 0xffff_ffff, None);
            }
            InputKind::Select => {
                pfb.fill_rect(x, y, w, h, 0xff1f_1f1f);
                frame(pfb, border);
                pfb.draw_string(x + 4, y + 3, &f.value, 0xffe8_e8e8, None);
                if w > 14 {
                    pfb.draw_string(x + w - 12, y + 3, "v", 0xff90_98a8, None);
                }
            }
            _ => {
                pfb.fill_rect(x, y, w, h, 0xff1f_1f1f);
                frame(pfb, border);
                let shown = if f.kind == InputKind::Password {
                    "*".repeat(f.value.chars().count())
                } else {
                    f.value.clone()
                };
                let txt = if focused { format!("{shown}_") } else { shown };
                pfb.draw_string(x + 4, y + 3, &txt, 0xffe8_e8e8, None);
            }
        }
    }
}

/// Repaint the current band (controls included) then the view. Used after a
/// field state change — no re-fetch or re-layout, just re-rasterize.
fn paint_fields(win: &mut Window) {
    repaint_band(win);
    paint_view(win);
}

/// Rasterize the band of the document around `band_top` into the page buffer:
/// (re)allocate it to min(doc_h, BAND_H) rows, clear it, then paint the retained
/// display list and the field overlays offset by -band_top.
fn repaint_band(win: &mut Window) {
    let (view_w, doc_h, band_top, page_bg) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        (st.page_w, st.doc_h, st.band_top, st.page_bg)
    };
    if view_w == 0 || doc_h == 0 {
        return;
    }
    // (Re)allocate the band buffer if its size is wrong, shrinking the height
    // until the (fragmented) heap can give us a contiguous slab.
    let target = doc_h.min(BAND_H).max(1);
    {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        if st.page.len() != view_w * target {
            let mut band_h = target;
            st.page = loop {
                let mut v: Vec<u32> = Vec::new();
                if v.try_reserve_exact(view_w * band_h).is_ok() {
                    v.resize(view_w * band_h, page_bg);
                    break v;
                }
                if band_h <= 300 {
                    v.resize(view_w * band_h, page_bg);
                    break v;
                }
                band_h = band_h * 2 / 3;
            };
        }
    }
    let crate::wm::App::Browser(st) = &mut win.app else { return };
    let bh = (st.page.len() / view_w.max(1)) as isize;
    let top = band_top as isize;
    // SAFETY: st.page is exactly view_w * bh; pfb writes through a raw pointer so
    // the &st.items / &st.imgs reads below don't alias it as far as the model.
    let pfb = unsafe { Framebuffer::new(st.page.as_mut_ptr(), view_w, bh as usize, view_w * 4) };
    pfb.clear(page_bg);
    for item in &st.items {
        match item {
            &Item::Rect { x, y, w, h, color } => {
                if w <= 0 || h <= 0 {
                    continue;
                }
                let yy = y - top;
                if yy + h <= 0 || yy >= bh {
                    continue;
                }
                let (dy, dh) = if yy < 0 { (0isize, h + yy) } else { (yy, h) };
                if dh > 0 {
                    pfb.fill_rect(x.max(0) as usize, dy as usize, w as usize, dh as usize, color);
                }
            }
            Item::Text { x, y, s, color, font, .. } => {
                let yy = *y - top;
                if *y >= 0 && yy >= 0 && yy < bh {
                    let color = readable(*color, page_bg);
                    pfb.draw_text((*x).max(0) as usize, yy as usize, s, font.id, font.px, color);
                }
            }
            &Item::Image { x, y, w, h, idx } => {
                let dy = y - top;
                if dy + h <= 0 || dy >= bh {
                    continue;
                }
                match st.imgs.get(idx) {
                    Some(Some(img)) if img.w as isize == w && img.h as isize == h => {
                        pfb.blit(x, dy, &img.pixels, img.w, img.h);
                    }
                    Some(Some(img)) => {
                        // Decoded size differs from the laid-out box (deferred
                        // image): nearest-neighbour scale into the box.
                        pfb.blit_scaled(x, dy, w, h, &img.pixels, img.w, img.h);
                    }
                    _ => {
                        // Not fetched yet: a light placeholder box so layout is
                        // stable and the user sees something until it loads.
                        if w > 1 && h > 1 {
                            let (py, ph) = if dy < 0 { (0isize, h + dy) } else { (dy, h) };
                            if ph > 0 {
                                pfb.fill_rect(x.max(0) as usize, py as usize, w as usize, ph as usize, 0xff26_2a30);
                                pfb.fill_rect(x.max(0) as usize, py as usize, w as usize, 1, 0xff3a_4048);
                            }
                        }
                    }
                }
            }
        }
    }
    let (fields, focus) = (st.fields.clone(), st.focus);
    paint_field_overlays(&pfb, &fields, focus, top, bh);
    kprintln!("BROWSER: band rasterized top={band_top} h={bh} (doc {doc_h})");
}

/// Make sure the rasterized band covers the current scroll window; if not,
/// re-center the band on the viewport and repaint it.
fn ensure_band(win: &mut Window, view_h: usize) {
    let (scroll, doc_h, band_top, view_w, plen) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        (st.scroll, st.doc_h, st.band_top, st.page_w, st.page.len())
    };
    if view_w == 0 || doc_h == 0 {
        return;
    }
    let bh = plen / view_w;
    if bh > 1 && scroll >= band_top && scroll + view_h <= band_top + bh {
        return; // viewport already inside the band
    }
    let target_bh = doc_h.min(BAND_H);
    let new_top = scroll
        .saturating_sub(target_bh.saturating_sub(view_h) / 2)
        .min(doc_h.saturating_sub(target_bh));
    if let crate::wm::App::Browser(st) = &mut win.app {
        st.band_top = new_top;
    }
    repaint_band(win);
}

/// Click in the chrome (topbar): focus the address bar for editing if the
/// click landed on the URL field (right of the back button). Returns true if
/// the click was consumed by the chrome.
pub fn chrome_click(win: &mut Window, rx: isize, ry: isize) -> bool {
    let crate::wm::App::Browser(st) = &mut win.app else { return false };
    // The URL field is in the address-bar row, right of the back/forward buttons.
    if ry >= TABBAR_H as isize && (ry as usize) < CHROME && rx >= 36 {
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

/// Host portion of an absolute URL (no scheme/port/path), lowercased. For local
/// paths returns "veil".
fn url_host(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(r) => {
            let hp = r.split(['/', '?', '#']).next().unwrap_or(r);
            hp.split(':').next().unwrap_or(hp).to_ascii_lowercase()
        }
        None => String::from("veil"),
    }
}

// --- cookie jar (session-scoped, not persisted) -----------------------------

struct Cookie {
    domain: String,
    name: String,
    value: String,
}

static mut COOKIE_JAR: Vec<Cookie> = Vec::new();

/// Store cookies from one `Set-Cookie` header value for `req_host`.
fn store_cookie(req_host: &str, set_cookie: &str) {
    // "name=value; Domain=...; Path=...; HttpOnly; ..." — keep name=value + Domain.
    let mut parts = set_cookie.split(';');
    let Some(nv) = parts.next() else { return };
    let Some((name, value)) = nv.split_once('=') else { return };
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() {
        return;
    }
    let mut domain = req_host.to_ascii_lowercase();
    for attr in parts {
        let a = attr.trim();
        if let Some(d) = a.strip_prefix("Domain=").or_else(|| a.strip_prefix("domain=")) {
            domain = d.trim().trim_start_matches('.').to_ascii_lowercase();
        }
    }
    let jar = unsafe { &mut *core::ptr::addr_of_mut!(COOKIE_JAR) };
    if let Some(c) = jar.iter_mut().find(|c| c.domain == domain && c.name == name) {
        c.value = String::from(value);
    } else {
        jar.push(Cookie { domain, name: String::from(name), value: String::from(value) });
        if jar.len() > 200 {
            jar.remove(0);
        }
    }
}

/// The `Cookie:` header value for a request to `host` ("a=1; b=2"), or empty.
fn cookie_header(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    let jar = unsafe { &*core::ptr::addr_of!(COOKIE_JAR) };
    let mut out = String::new();
    for c in jar {
        if host == c.domain || host.ends_with(&alloc::format!(".{}", c.domain)) {
            if !out.is_empty() {
                out.push_str("; ");
            }
            out.push_str(&c.name);
            out.push('=');
            out.push_str(&c.value);
        }
    }
    out
}

/// Scan a raw HTTP response's header block for `Set-Cookie:` lines and store them.
fn harvest_cookies(resp: &[u8], host: &str) {
    let Some(end) = header_end(resp) else { return };
    let head = core::str::from_utf8(&resp[..end]).unwrap_or("");
    for line in head.split("\r\n") {
        if let Some(v) = line.strip_prefix("Set-Cookie:").or_else(|| line.strip_prefix("set-cookie:")) {
            store_cookie(host, v.trim());
        }
    }
}

/// Build a request (headers + optional body). `body` present ⇒ POST with
/// `application/x-www-form-urlencoded`. Adds a matching `Cookie:` header.
fn build_request(path: &str, host: &str, body: Option<&[u8]>) -> Vec<u8> {
    let method = if body.is_some() { "POST" } else { "GET" };
    let mut h = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: VeilOS\r\nAccept: text/html\r\n");
    let ck = cookie_header(host);
    if !ck.is_empty() {
        h.push_str(&format!("Cookie: {ck}\r\n"));
    }
    if let Some(b) = body {
        h.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        h.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    h.push_str("Connection: close\r\n\r\n");
    let mut out = h.into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(b);
    }
    out
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
fn tls_get(url: &str, body: Option<&[u8]>) -> Option<(u32, String, Vec<u8>)> {
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
    let req = build_request(path, host, body);
    conn.write(&req);
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
    harvest_cookies(&resp, host);
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
fn http_direct(url: &str, body: Option<&[u8]>) -> Option<(u32, String, Vec<u8>)> {
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
    let req = build_request(path, host, body);
    write_all(h, &req);
    let resp = read_http(h, 1 << 20, FETCH_TIMEOUT, FETCH_TIMEOUT * 4);
    net::tcp_close(h);
    harvest_cookies(&resp, host);
    let parsed = parse_response(&resp, url)?;
    if parsed.0 == 200 && !DIRECT_HTTP_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("DIRECT_HTTP_OK: fetched {host} over kernel TCP (no host proxy)");
    }
    Some(parsed)
}

/// GET `path` (no body).
fn http_get(path: &str) -> Option<(u32, String, Vec<u8>)> {
    http_request(path, None)
}

/// POST `body` to `path` as application/x-www-form-urlencoded.
fn http_post(path: &str, body: &[u8]) -> Option<(u32, String, Vec<u8>)> {
    http_request(path, Some(body))
}

/// Fetch `path`, optionally with a POST `body`. Local paths ("/page.htm") hit
/// our own HTTP server on loopback; `https://` URLs use the from-scratch TLS 1.3
/// stack directly; other external `http://` URLs go direct over our TCP stack,
/// then the host proxy at 10.0.2.2:7779 as a fallback.
fn http_request(path: &str, body: Option<&[u8]>) -> Option<(u32, String, Vec<u8>)> {
    if is_https(path) {
        if let Some(r) = tls_get(path, body) {
            return Some(r);
        }
        kprintln!("BROWSER: direct TLS failed for {path}, falling back to proxy");
    } else if is_external(path) {
        if let Some(r) = http_direct(path, body) {
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
    let host = if external { url_host(path) } else { String::from("veil") };
    for attempt in 0..2 {
        if attempt > 0 {
            for _ in 0..10 {
                scheduler::yield_now();
            }
        }
        let Some(h) = net::tcp_connect(ip, port) else { continue };
        // The proxy expects an absolute-form request line for external URLs; the
        // local server a normal path. build_request keeps Host = the real host
        // so cookies are scoped correctly.
        let req = build_request(path, &host, body);
        write_all(h, &req);
        let resp = read_http(h, 1 << 20, timeout, timeout * 4);
        net::tcp_close(h);
        harvest_cookies(&resp, &host);
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
    // Local site: drop only the fragment (keep the query — GET forms need it)
    // and normalise to an absolute path.
    let href = href.split('#').next().unwrap_or("");
    if href.starts_with('/') {
        String::from(href)
    } else {
        format!("/{href}")
    }
}

/// Fetch + decode the image in slot `i`, updating its laid-out size to the
/// decoded dimensions and inserting it into the LRU cache. Returns true on a
/// successful decode.
fn fetch_image_slot(
    slots: &mut [ImgSlot],
    pixels: &mut [Option<png::Image>],
    i: usize,
    cache: &mut Vec<(String, png::Image)>,
) -> bool {
    let src = slots[i].src.clone();
    if let Some((200, _, data)) = http_get(&src) {
        if let Some(img) = png::decode_any(&data) {
            kprintln!("BROWSER: decoded {src} ({}x{} px)", img.w, img.h);
            if is_external(&src) && !EXT_IMG_DONE.swap(true, Ordering::Relaxed) {
                kprintln!("EXT_IMG_OK");
            }
            slots[i].w = img.w as isize;
            slots[i].h = img.h as isize;
            cache.insert(0, (src, img.clone()));
            cache.truncate(IMG_CACHE_CAP);
            pixels[i] = Some(img);
            return true;
        }
        kprintln!("BROWSER: {src} is not a PNG (skipped)");
    }
    false
}

/// Lazily fetch any deferred images that have scrolled within ~1 viewport of the
/// visible window, then repaint if anything loaded. Called on scroll. Deferred
/// images keep their laid-out box, so the decoded pixels are scaled into it (no
/// re-layout). Returns true if at least one image loaded.
fn lazy_load_images(win: &mut Window, view_h: usize) -> bool {
    // Collect the deferred slots now near the viewport (item index -> slot).
    let to_fetch: Vec<usize> = {
        let crate::wm::App::Browser(st) = &win.app else { return false };
        let lo = st.scroll.saturating_sub(view_h) as isize;
        let hi = (st.scroll + 2 * view_h) as isize;
        let mut want: Vec<usize> = Vec::new();
        for it in &st.items {
            if let &Item::Image { y, h, idx, .. } = it {
                let visible = y + h >= lo && y <= hi;
                if visible
                    && matches!(st.imgs.get(idx), Some(None))
                    && !want.contains(&idx)
                {
                    want.push(idx);
                }
            }
        }
        want
    };
    if to_fetch.is_empty() {
        return false;
    }
    // Fetch each, updating the per-slot pixels + decoded size in place.
    let mut cache = match &mut win.app {
        crate::wm::App::Browser(st) => core::mem::take(&mut st.img_cache),
        _ => return false,
    };
    let mut any = false;
    for idx in to_fetch {
        let (src, mut img) = {
            let crate::wm::App::Browser(st) = &win.app else { break };
            (st.img_src.get(idx).cloned().unwrap_or_default(), None)
        };
        if src.is_empty() {
            continue;
        }
        // Cache hit or fetch.
        if let Some(pos) = cache.iter().position(|(s, _)| *s == src) {
            img = Some(cache[pos].1.clone());
        } else if let Some((200, _, data)) = http_get(&src) {
            if let Some(decoded) = png::decode_any(&data) {
                kprintln!("BROWSER: lazy-decoded {src} ({}x{} px)", decoded.w, decoded.h);
                cache.insert(0, (src.clone(), decoded.clone()));
                cache.truncate(IMG_CACHE_CAP);
                img = Some(decoded);
            }
        }
        if let Some(decoded) = img {
            if let crate::wm::App::Browser(st) = &mut win.app {
                if let Some(slot) = st.imgs.get_mut(idx) {
                    *slot = Some(decoded);
                    any = true;
                }
            }
        }
    }
    if let crate::wm::App::Browser(st) = &mut win.app {
        st.img_cache = cache;
    }
    if any {
        repaint_band(win);
    }
    any
}

/// Compatibility shim for sites whose default front end is a JS-only SPA we
/// can't run: rewrite them to a server-rendered equivalent. Reddit's new site
/// renders blank without its React bundle, but old.reddit.com serves real HTML
/// (a readable post list), so transparently route reddit.com there.
fn compat_rewrite(url: &str) -> String {
    for prefix in ["https://www.reddit.com", "https://reddit.com", "http://www.reddit.com", "http://reddit.com"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("https://old.reddit.com{rest}");
        }
    }
    String::from(url)
}

/// Derive a FAT16 8.3 filename for a downloaded resource from its URL and, as a
/// fallback for the extension, its content type. e.g. "/docs/report.pdf?v=2" +
/// "application/pdf" -> "REPORT.PDF"; "/dl" + "application/zip" -> "DL.ZIP".
fn download_name(url: &str, ctype: &str) -> String {
    let tail = url.split(['?', '#']).next().unwrap_or(url);
    let seg = tail.rsplit('/').next().unwrap_or("").trim();
    let (base, ext) = match seg.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b, e),
        _ => (seg, ""),
    };
    let clean = |s: &str, n: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(n)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let mut b = clean(base, 8);
    if b.is_empty() {
        b = String::from("DOWNLOAD");
    }
    let mut e = clean(ext, 3);
    if e.is_empty() {
        // Map the content type's subtype to a sensible extension.
        let sub = ctype.split(['/', ';', '+']).nth(1).unwrap_or("").trim();
        e = match sub {
            "pdf" => "PDF",
            "zip" => "ZIP",
            "json" => "JSN",
            "jpeg" | "jpg" => "JPG",
            "png" => "PNG",
            "gif" => "GIF",
            "plain" => "TXT",
            "csv" => "CSV",
            "xml" => "XML",
            "mp4" => "MP4",
            "mpeg" | "mp3" => "MP3",
            _ => "BIN",
        }
        .into();
    }
    format!("{b}.{e}")
}

// --- style ----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Display {
    Block,
    Inline,
    None,
    Flex,
    Grid,
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
    // Grid container: number of columns (from grid-template-columns).
    grid_cols: usize,
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
        grid_cols: 1,
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
                "grid" | "inline-grid" => Display::Grid,
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
        "gap" | "grid-gap" | "grid-column-gap" | "column-gap" => {
            // gap may be "row col"; use the first (column gaps drive our grid).
            s.gap = parse_px(val.split_whitespace().next().unwrap_or(val)).unwrap_or(s.gap);
        }
        "grid-template-columns" => s.grid_cols = count_grid_columns(val),
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
        grid_cols: 1,
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
        | "figure" | "figcaption" | "blockquote" | "form" => s.display = Display::Block,
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
    let zoom = ZOOM.load(Ordering::Relaxed).max(50) as usize;
    s.font = Font {
        id: pick_ftid(s.font_fam, s.font_weight, s.font_italic),
        px: ((s.scale * 16 * zoom / 100).max(10) as u16).min(200),
    };
    s
}

// --- layout ---------------------------------------------------------------------

enum Item {
    Rect { x: isize, y: isize, w: isize, h: isize, color: u32 },
    Text { x: isize, y: isize, s: String, color: u32, scale: usize, font: Font },
    Image { x: isize, y: isize, w: isize, h: isize, idx: usize },
}

enum Frag {
    Word { s: String, color: u32, scale: usize, link: Option<String>, underline: bool, font: Font },
    Space { scale: usize, font: Font },
    Img { idx: usize, w: isize, h: isize },
    Input {
        name: String,
        value: String,
        kind: InputKind,
        checked: bool,
        w: isize,
        h: isize,
        multiline: bool,
        options: Vec<String>,
    },
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

/// One `<img>` slot: its source and the box it occupies in layout (the decoded
/// size once fetched, otherwise the HTML width/height attrs or a default).
struct ImgSlot {
    src: String,
    w: isize,
    h: isize,
}

struct Ctx<'a> {
    sheet: &'a [css::Rule],
    imgs: &'a [ImgSlot],
    items: Vec<Item>,
    links: Vec<LinkBox>,
    img_spots: Vec<(usize, isize, isize)>, // (imgs idx, x, y) for the proof log
    fields: Vec<InputField>,               // on-page form fields, with positions
    forms: Vec<Form>,                      // forms encountered during layout
    cur_form: usize,                       // current enclosing form (MAX = none)
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
                    let ty = node.attr("type").unwrap_or("text").to_ascii_lowercase();
                    let name = node.attr("name").unwrap_or("").into();
                    let value: String = node.attr("value").unwrap_or("").into();
                    let checked = node.attr("checked").is_some();
                    match ty.as_str() {
                        "checkbox" => buf.push(Frag::Input {
                            name, value: if value.is_empty() { "on".into() } else { value },
                            kind: InputKind::Checkbox, checked, w: 16, h: 16, multiline: false, options: Vec::new(),
                        }),
                        "radio" => buf.push(Frag::Input {
                            name, value: if value.is_empty() { "on".into() } else { value },
                            kind: InputKind::Radio, checked, w: 16, h: 16, multiline: false, options: Vec::new(),
                        }),
                        "submit" | "button" => {
                            let label = if value.is_empty() { String::from("Submit") } else { value };
                            let bw = label.chars().count() as isize * 9 + 24;
                            buf.push(Frag::Input {
                                name, value: label, kind: InputKind::Submit, checked: false,
                                w: bw, h: 26, multiline: false, options: Vec::new(),
                            });
                        }
                        "hidden" => buf.push(Frag::Input {
                            name, value, kind: InputKind::Hidden, checked: false, w: 0, h: 0, multiline: false, options: Vec::new(),
                        }),
                        // text-like (incl. unknown types)
                        _ => {
                            let kind = if ty == "password" { InputKind::Password } else { InputKind::Text };
                            let chars = node.attr("size").and_then(|s| s.parse::<isize>().ok()).unwrap_or(20);
                            buf.push(Frag::Input {
                                name, value, kind, checked: false,
                                w: (chars * 8 + 8).clamp(80, 360), h: 20, multiline: false, options: Vec::new(),
                            });
                        }
                    }
                }
                "textarea" => {
                    let mut value = String::new();
                    node.text(&mut value);
                    buf.push(Frag::Input {
                        name: node.attr("name").unwrap_or("").into(),
                        value: value.trim().into(), kind: InputKind::Textarea, checked: false,
                        w: 280, h: 64, multiline: true, options: Vec::new(),
                    });
                }
                "select" => {
                    // Collect <option> labels; selected = the one with `selected`,
                    // else the first.
                    let mut options = Vec::new();
                    let mut value = String::new();
                    for opt in node.children() {
                        if opt.tag() == Some("option") {
                            let mut label = String::new();
                            opt.text(&mut label);
                            let label = label.trim().to_string();
                            let v = opt.attr("value").map(String::from).unwrap_or_else(|| label.clone());
                            if opt.attr("selected").is_some() || value.is_empty() {
                                value = v.clone();
                            }
                            // Store the submittable value; cycling/submission use it.
                            options.push(v);
                        }
                    }
                    buf.push(Frag::Input {
                        name: node.attr("name").unwrap_or("").into(),
                        value, kind: InputKind::Select, checked: false,
                        w: 160, h: 22, multiline: false, options,
                    });
                }
                "button" => {
                    let ty = node.attr("type").unwrap_or("submit").to_ascii_lowercase();
                    if ty != "button" {
                        let mut label = String::new();
                        node.text(&mut label);
                        let label = if label.trim().is_empty() { String::from("Submit") } else { label.trim().into() };
                        let bw = label.chars().count() as isize * 9 + 24;
                        buf.push(Frag::Input {
                            name: node.attr("name").unwrap_or("").into(),
                            value: label, kind: InputKind::Submit, checked: false,
                            w: bw, h: 26, multiline: false, options: Vec::new(),
                        });
                    }
                }
                "img" => {
                    let src = node.attr("src").map(resolve_href).unwrap_or_default();
                    // Every <img> with a known slot reserves its box (so deferred,
                    // not-yet-fetched images still take up space and can be
                    // lazy-loaded into place on scroll).
                    if let Some(idx) = ctx.imgs.iter().position(|s| s.src == src) {
                        let slot = &ctx.imgs[idx];
                        buf.push(Frag::Img { idx, w: slot.w, h: slot.h });
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
            Frag::Img { idx, w: fw, h: fh } => {
                ctx.img_spots.push((idx, x + dx, fy));
                ctx.items.push(Item::Image { x: x + dx, y: fy, w: fw, h: fh, idx });
            }
            Frag::Input { name, value, kind, checked, w: fw, h: fh, multiline, options } => {
                let (bx, by) = (x + dx, fy);
                let ui = Font { id: crate::freetype::FontId::Ui, px: 16 };
                match kind {
                    InputKind::Hidden => {} // not rendered, but submittable
                    InputKind::Checkbox => {
                        ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff2a2a2a });
                        for (rx, ry, rw, rh) in [(bx, by, fw, 1), (bx, by + fh - 1, fw, 1), (bx, by, 1, fh), (bx + fw - 1, by, 1, fh)] {
                            ctx.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: 0xff8a90a0 });
                        }
                        if checked {
                            ctx.items.push(Item::Rect { x: bx + 3, y: by + 3, w: fw - 6, h: fh - 6, color: 0xff5b8af0 });
                        }
                    }
                    InputKind::Radio => {
                        ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff2a2a2a });
                        for (rx, ry, rw, rh) in [(bx, by, fw, 1), (bx, by + fh - 1, fw, 1), (bx, by, 1, fh), (bx + fw - 1, by, 1, fh)] {
                            ctx.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: 0xff8a90a0 });
                        }
                        if checked {
                            ctx.items.push(Item::Rect { x: bx + 4, y: by + 4, w: fw - 8, h: fh - 8, color: 0xff5b8af0 });
                        }
                    }
                    InputKind::Submit => {
                        ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff3a6ea5 });
                        ctx.items.push(Item::Text { x: bx + 10, y: by + 5, s: value.clone(), color: 0xffffffff, scale: 1, font: ui });
                    }
                    InputKind::Select => {
                        ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff1f1f1f });
                        for (rx, ry, rw, rh) in [(bx, by, fw, 1), (bx, by + fh - 1, fw, 1), (bx, by, 1, fh), (bx + fw - 1, by, 1, fh)] {
                            ctx.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: 0xff4a5060 });
                        }
                        ctx.items.push(Item::Text { x: bx + 4, y: by + 3, s: value.clone(), color: 0xffe8e8e8, scale: 1, font: ui });
                        ctx.items.push(Item::Text { x: bx + fw - 14, y: by + 3, s: String::from("v"), color: 0xff9098a8, scale: 1, font: ui });
                    }
                    _ => {
                        // text / password / textarea
                        ctx.items.push(Item::Rect { x: bx, y: by, w: fw, h: fh, color: 0xff1f1f1f });
                        for (rx, ry, rw, rh) in [(bx, by, fw, 1), (bx, by + fh - 1, fw, 1), (bx, by, 1, fh), (bx + fw - 1, by, 1, fh)] {
                            ctx.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: 0xff4a5060 });
                        }
                        let shown = if kind == InputKind::Password {
                            "*".repeat(value.chars().count())
                        } else {
                            value.clone()
                        };
                        ctx.items.push(Item::Text { x: bx + 4, y: by + 3, s: shown, color: 0xffe8e8e8, scale: 1, font: ui });
                    }
                }
                ctx.fields.push(InputField {
                    x: bx, y: by, w: fw, h: fh, name, value, kind, checked, multiline,
                    form: ctx.cur_form, options,
                });
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
    // Entering a <form>: register it and scope its inputs to it.
    let prev_form = ctx.cur_form;
    if node.tag() == Some("form") {
        let method = node.attr("method").unwrap_or("GET").to_ascii_uppercase();
        let action = node.attr("action").map(resolve_href).unwrap_or_else(|| page_base().unwrap_or_else(|| String::from("/")));
        ctx.cur_form = ctx.forms.len();
        ctx.forms.push(Form { method, action });
    }
    y += style.margin[0];
    let bx = x + style.margin[3];
    let bw = style
        .width
        .unwrap_or(w - style.margin[3] - style.margin[1])
        .max(16);
    if node.tag() == Some("table") {
        let r = layout_table(ctx, node, &style, bx, bw, y) + style.margin[2];
        ctx.cur_form = prev_form;
        return r;
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
    } else if style.display == Display::Grid {
        cy = layout_grid(ctx, node, &style, cx, cw, cy);
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
    ctx.cur_form = prev_form;
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
            && matches!(cstyle.display, Display::Block | Display::Flex | Display::Grid);
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

// --- CSS grid ---------------------------------------------------------------

static GRID_DONE: AtomicBool = AtomicBool::new(false);

/// Count the column tracks in a `grid-template-columns` value, expanding
/// `repeat(N, ...)`. e.g. "1fr 1fr" -> 2, "repeat(3, minmax(0,1fr))" -> 3,
/// "repeat(auto-fill, 200px)" -> a sensible default.
fn count_grid_columns(val: &str) -> usize {
    let v = val.trim();
    if let Some(rest) = v.strip_prefix("repeat(") {
        // repeat(N, track...) — N is the first arg.
        let inner = rest.trim_end_matches(')');
        let first = inner.split(',').next().unwrap_or("").trim();
        if let Ok(n) = first.parse::<usize>() {
            return n.max(1);
        }
        return 2; // auto-fill / auto-fit: assume 2 columns
    }
    // Count top-level track tokens (ignore commas inside minmax()/functions).
    let mut depth = 0;
    let mut count = 0;
    let mut in_tok = false;
    for c in v.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ' ' | '\t' if depth == 0 => in_tok = false,
            _ if depth == 0 => {
                if !in_tok {
                    count += 1;
                    in_tok = true;
                }
            }
            _ => {}
        }
    }
    count.max(1)
}

/// Lay out a `display:grid` container: `grid_cols` equal-width columns, items
/// flowing row-major, each row sized to its tallest item, `gap` between tracks.
fn layout_grid(ctx: &mut Ctx, node: &html::Node, style: &Style, x: isize, w: isize, y: isize) -> isize {
    if !GRID_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("GRID_OK");
    }
    let cols = style.grid_cols.max(1);
    let gap = style.gap;
    let col_w = ((w - gap * (cols as isize - 1)) / cols as isize).max(1);

    // Element children are the grid items.
    let items: Vec<&html::Node> = node
        .children()
        .iter()
        .filter(|c| {
            matches!(c, html::Node::Element { .. }) && resolve(ctx.sheet, c, style).display != Display::None
        })
        .collect();
    if items.is_empty() {
        return y;
    }

    let mut cy = y;
    let mut col = 0usize;
    let mut row_h = 0isize;
    let mut row_start = cy;
    for it in items {
        if col == cols {
            col = 0;
            cy = row_start + row_h + gap;
            row_start = cy;
            row_h = 0;
        }
        let cx = x + col as isize * (col_w + gap);
        // measure each item at its column width and place it
        let (cells, links, spots, _, ch) = measure_item(ctx, it, style, col_w);
        place(ctx, cells, links, spots, cx, row_start);
        row_h = row_h.max(ch);
        col += 1;
    }
    row_start + row_h
}

// --- flexbox ----------------------------------------------------------------

static FLEX_DONE: AtomicBool = AtomicBool::new(false);

fn item_right(it: &Item, ctx: &Ctx) -> isize {
    match it {
        Item::Text { x, s, scale, font, .. } => x + text_w(s, *scale, *font),
        Item::Image { x, w, .. } => x + w,
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
        forms: Vec::new(),
        cur_form: usize::MAX,
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

/// Navigate back in the active tab's history, pushing the current page onto
/// the forward stack.
pub fn back(win: &mut Window) -> bool {
    let prev = match &mut win.app {
        crate::wm::App::Browser(st) => {
            let a = st.active;
            let cur = st.path.clone();
            match st.tabs[a].back.pop() {
                Some(p) => {
                    st.tabs[a].fwd.push(cur);
                    Some(p)
                }
                None => None,
            }
        }
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

/// Navigate forward (re-do a back), pushing the current page onto the back stack.
pub fn forward(win: &mut Window) -> bool {
    let next = match &mut win.app {
        crate::wm::App::Browser(st) => {
            let a = st.active;
            let cur = st.path.clone();
            match st.tabs[a].fwd.pop() {
                Some(p) => {
                    st.tabs[a].back.push(cur);
                    Some(p)
                }
                None => None,
            }
        }
        _ => return false,
    };
    let Some(p) = next else { return false };
    kprintln!("BROWSER: forward to {p}");
    navigate(win, &p, false);
    true
}

/// Reload the current page (F5).
pub fn reload(win: &mut Window) {
    let path = if let crate::wm::App::Browser(st) = &win.app { st.path.clone() } else { return };
    navigate(win, &path, false);
}

/// A short label for the tab strip from a path/URL.
fn tab_title(path: &str) -> String {
    let p = path.trim_end_matches('/');
    let p = p.strip_prefix("https://").or_else(|| p.strip_prefix("http://")).unwrap_or(p);
    let name = p.rsplit('/').find(|s| !s.is_empty()).unwrap_or(p);
    let name = name.split('?').next().unwrap_or(name);
    if name.is_empty() {
        String::from("new tab")
    } else {
        name.chars().take(18).collect()
    }
}

// --- M39 tabs + zoom --------------------------------------------------------

/// Open `path` in a new tab and switch to it.
pub fn new_tab(win: &mut Window, path: &str) {
    {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        // Save the current tab's scroll before leaving it.
        let a = st.active;
        st.tabs[a].scroll = st.scroll;
        st.tabs.push(Tab::new(path));
        st.active = st.tabs.len() - 1;
        st.scroll = 0;
    }
    kprintln!("BROWSER: new tab -> {path}");
    navigate(win, path, false);
}

/// Close the active tab (Ctrl+W).
pub fn close_tab(win: &mut Window) {
    let a = if let crate::wm::App::Browser(st) = &win.app { st.active } else { return };
    close_tab_index(win, a);
}

/// Close tab `i`; switch to a neighbour. Keeps at least one tab (the last one
/// resets to the home page rather than closing the window).
pub fn close_tab_index(win: &mut Window, i: usize) {
    let target = {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        if i >= st.tabs.len() {
            return;
        }
        if st.tabs.len() <= 1 {
            st.tabs[0] = Tab::new("/");
            st.active = 0;
            st.scroll = 0;
            String::from("/")
        } else {
            st.tabs.remove(i);
            if st.active >= st.tabs.len() {
                st.active = st.tabs.len() - 1;
            } else if i < st.active {
                st.active -= 1;
            }
            st.scroll = st.tabs[st.active].scroll;
            st.tabs[st.active].path.clone()
        }
    };
    kprintln!("BROWSER: closed tab {i}, now on {target}");
    navigate(win, &target, false);
}

const TAB_MAX_W: usize = 150;
const NEWTAB_W: usize = 22;

/// Geometry of the tab strip: per-tab (x, width) cells + the new-tab button.
fn tab_layout(ntabs: usize, cw: usize) -> (Vec<(usize, usize)>, (usize, usize)) {
    let avail = cw.saturating_sub(NEWTAB_W + 6);
    let tw = (avail / ntabs.max(1)).clamp(40, TAB_MAX_W);
    let cells: Vec<(usize, usize)> = (0..ntabs).map(|i| (i * tw, tw)).collect();
    let nt_x = (ntabs * tw).min(cw.saturating_sub(NEWTAB_W));
    (cells, (nt_x, NEWTAB_W))
}

/// Route a click in the chrome (tab strip + back/forward buttons). Returns true
/// if consumed. (rx, ry) are content-relative.
pub fn topbar_click(win: &mut Window, rx: isize, ry: isize) -> bool {
    if rx < 0 || ry < 0 {
        return false;
    }
    let (rx, ry) = (rx as usize, ry as usize);
    if ry < TABBAR_H {
        let (ntabs, cw) = match &win.app {
            crate::wm::App::Browser(st) => (st.tabs.len(), win.cw),
            _ => return false,
        };
        let (cells, (nt_x, nt_w)) = tab_layout(ntabs, cw);
        if rx >= nt_x && rx < nt_x + nt_w {
            new_tab(win, "/");
            return true;
        }
        for (i, &(tx, tw)) in cells.iter().enumerate() {
            if rx >= tx && rx < tx + tw {
                if rx >= tx + tw - 16 {
                    close_tab_index(win, i);
                } else {
                    switch_tab(win, i);
                }
                return true;
            }
        }
        return true; // empty strip area
    }
    if ry >= TABBAR_H && ry < CHROME {
        if rx < 18 {
            back(win);
            return true;
        }
        if rx < 36 {
            forward(win);
            return true;
        }
    }
    false
}

/// Switch to tab `i`, re-rendering its page.
pub fn switch_tab(win: &mut Window, i: usize) {
    let target = {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        if i >= st.tabs.len() || i == st.active {
            return;
        }
        let a = st.active;
        st.tabs[a].scroll = st.scroll; // remember where we were
        st.active = i;
        st.scroll = st.tabs[i].scroll;
        st.tabs[i].path.clone()
    };
    kprintln!("BROWSER: switch to tab {i} ({target})");
    navigate(win, &target, false);
    // Restore the saved scroll after the re-render.
    if let crate::wm::App::Browser(st) = &mut win.app {
        let s = st.tabs[st.active].scroll.min(st.doc_h.saturating_sub(1));
        st.scroll = s;
    }
    paint_view(win);
}

/// Change zoom by `delta` percent (0 = reset to 100), then re-render.
pub fn zoom(win: &mut Window, delta: i32) {
    let path = {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        st.zoom = if delta == 0 {
            100
        } else {
            (st.zoom as i32 + delta).clamp(50, 250) as u16
        };
        st.path.clone()
    };
    kprintln!("BROWSER: zoom {}%", zoom_pct(win));
    navigate(win, &path, false);
}

fn zoom_pct(win: &Window) -> u16 {
    if let crate::wm::App::Browser(st) = &win.app { st.zoom } else { 100 }
}

struct FontFace {
    family: String,
    weight: u16,
    italic: bool,
    url: String,
}

/// Parse `@font-face` blocks from raw CSS text (css::parse skips them).
fn parse_font_faces(css: &str) -> Vec<FontFace> {
    let mut out = Vec::new();
    let lower = css.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("@font-face") {
        let start = search + rel;
        let Some(open) = css[start..].find('{').map(|i| start + i) else { break };
        let Some(close) = css[open..].find('}').map(|i| open + i) else { break };
        let block = &css[open + 1..close];
        search = close + 1;
        let mut family = String::new();
        let mut weight = 400u16;
        let mut italic = false;
        let mut url = String::new();
        for decl in block.split(';') {
            let Some(c) = decl.find(':') else { continue };
            let prop = decl[..c].trim().to_ascii_lowercase();
            let val = decl[c + 1..].trim();
            match prop.as_str() {
                "font-family" => family = val.trim_matches(|ch| ch == '"' || ch == '\'').to_string(),
                "font-weight" => {
                    weight = val.split_whitespace().next().and_then(|w| w.parse().ok()).unwrap_or(400)
                }
                "font-style" => italic = val.contains("italic") || val.contains("oblique"),
                "src" => {
                    // pick the first url(...) (our UA yields a single .ttf)
                    if let Some(u) = val.find("url(") {
                        let rest = &val[u + 4..];
                        if let Some(end) = rest.find(')') {
                            url = rest[..end].trim().trim_matches(|c| c == '"' || c == '\'').to_string();
                        }
                    }
                }
                _ => {}
            }
        }
        if !family.is_empty() && !url.is_empty() {
            out.push(FontFace { family, weight, italic, url });
        }
    }
    out
}

/// Fetch + register the page's web fonts, bounded (a handful of TTFs) to keep
/// memory and fetch time in check. Prioritises one normal + one bold weight
/// per family.
fn register_font_faces(css: &str) {
    let faces = parse_font_faces(css);
    if faces.is_empty() {
        return;
    }
    // Prioritise: normal style, weights near 400 then near 700; one per
    // (family, weight-bucket, italic).
    let mut chosen: Vec<&FontFace> = Vec::new();
    let mut taken: Vec<(String, u8, bool)> = Vec::new();
    let bucket = |w: u16| -> u8 { if w >= 600 { 1 } else { 0 } };
    // pass 1: normal style; pass 2: italic — so normal wins the cap budget.
    for want_italic in [false, true] {
        for f in &faces {
            if chosen.len() >= 6 {
                break;
            }
            if f.italic != want_italic {
                continue;
            }
            // We can only decode TrueType/OpenType (not WOFF2/Brotli); skip the
            // rest so a woff2 @font-face doesn't claim a family's slot before the
            // TTF one Google Fonts serves our generic UA.
            let u = f.url.split('?').next().unwrap_or(&f.url).to_ascii_lowercase();
            if !(u.ends_with(".ttf") || u.ends_with(".otf")) {
                continue;
            }
            let key = (f.family.to_ascii_lowercase(), bucket(f.weight), f.italic);
            if taken.contains(&key) {
                continue;
            }
            taken.push(key);
            chosen.push(f);
        }
    }
    let mut loaded = 0;
    for f in chosen {
        let url = resolve_href(&f.url);
        if let Some((200, _, ttf)) = http_get(&url) {
            // sanity: a TTF/OTF starts with 0x00010000, "OTTO", "true", or "ttcf".
            let magic_ok = ttf.len() > 4
                && matches!(&ttf[..4], [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf");
            if magic_ok {
                if crate::freetype::register_web_font(&f.family, f.weight, f.italic, ttf).is_some() {
                    loaded += 1;
                }
            }
        }
    }
    if loaded > 0 {
        kprintln!("BROWSER: registered {loaded} web font(s)");
    }
}

/// Collect runnable script sources from `doc` in document order: inline
/// `<script>` bodies and same-origin (relative-src) external scripts. Skips
/// cross-origin absolute-URL scripts (analytics beacons) and non-JS types.
fn collect_scripts(doc: &html::Node) -> Vec<String> {
    let mut nodes = Vec::new();
    doc.find_all("script", &mut nodes);
    let mut out = Vec::new();
    for s in nodes {
        if let Some(t) = s.attr("type") {
            let t = t.to_ascii_lowercase();
            if !t.is_empty() && !t.contains("javascript") && t != "module" {
                continue; // application/ld+json, importmap, etc.
            }
        }
        match s.attr("src") {
            None => {
                let mut src = String::new();
                s.text(&mut src);
                if !src.trim().is_empty() {
                    out.push(src);
                }
            }
            Some(src) => {
                // Cross-origin absolute URLs (analytics) — skip.
                if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("//") {
                    continue;
                }
                if let Some((200, _, body)) = http_get(&resolve_href(src)) {
                    out.push(String::from_utf8_lossy(&body).into_owned());
                }
            }
        }
    }
    out
}

pub fn navigate(win: &mut Window, path: &str, by_click: bool) {
    navigate_body(win, path, None, by_click);
}

/// Navigate, optionally POSTing `body` (form submission). The body is sent only
/// on the first request; any 302/303 redirect is then followed with a GET.
pub fn navigate_body(win: &mut Window, path: &str, body: Option<Vec<u8>>, by_click: bool) {
    let path = compat_rewrite(&resolve_href(path));
    let was_external = is_external(&path);
    let path_for_log = path.clone();
    // Set the base so this page's relative stylesheets/images/links resolve
    // against its own host (external) or stay loopback-relative (local).
    set_page_base(if was_external { Some(path.clone()) } else { None });
    if let crate::wm::App::Browser(st) = &win.app {
        ZOOM.store(st.zoom, Ordering::Relaxed);
    }
    kprintln!("BROWSER: navigating to {path}");
    // A user navigation (link click / typed URL) pushes the current page onto
    // the history stack so the back button can return to it.
    if by_click {
        if let crate::wm::App::Browser(st) = &mut win.app {
            let old = st.path.clone();
            let a = st.active;
            if st.tabs[a].back.last() != Some(&old) {
                st.tabs[a].back.push(old);
                if st.tabs[a].back.len() > 30 {
                    st.tabs[a].back.remove(0);
                }
            }
            st.tabs[a].fwd.clear(); // a fresh navigation invalidates forward
        }
    }
    // Fetch, following up to a few 3xx redirects (Location may be relative).
    let mut path = path;
    let (status, ctype, body) = {
        let mut result = None;
        let mut pending_body = body; // POST body for the first request only
        for _ in 0..6 {
            let resp = http_request(&path, pending_body.as_deref());
            pending_body = None; // redirects are followed with GET
            let Some((s, c, b)) = resp else { break };
            if c == "text/redirect" {
                let loc = String::from_utf8_lossy(&b);
                let loc = loc.trim();
                let next = if is_external(loc) {
                    String::from(loc)
                } else if is_external(&path) {
                    url_join(&path, loc) // relative to the external page we're on
                } else {
                    resolve_href(loc) // local loopback path stays local
                };
                kprintln!("BROWSER: following redirect -> {next}");
                set_page_base(if is_external(&next) { Some(next.clone()) } else { None });
                path = compat_rewrite(&next);
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
        // Non-renderable response (PDF, zip, image, binary, ...) -> save it to
        // the filesystem as a download and toast, so it shows in the files app.
        let fname = download_name(&path, &ctype);
        match crate::fs::write_file(&fname, &body) {
            Ok(()) => {
                kprintln!("BROWSER: downloaded {path} ({} bytes) -> {fname}", body.len());
                crate::wm::queue_toast(format!("Downloaded {fname}"));
                render_message(
                    win,
                    &path,
                    &format!("Downloaded {fname} ({} bytes). Saved to disk — open it in Files.", body.len()),
                );
            }
            Err(()) => {
                kprintln!("BROWSER: download of {path} failed (write error)");
                render_message(win, &path, &format!("download failed: {fname} ({ctype})"));
            }
        }
        return;
    }
    let mut doc = html::parse(&String::from_utf8_lossy(&body));

    // JavaScript: collect inline + same-origin external <script> sources in
    // document order, run them against the DOM, and continue layout with the
    // (possibly heavily mutated) tree. Cross-origin scripts (analytics beacons)
    // are skipped. This is what makes JS-rendered pages actually show content.
    let scripts = collect_scripts(&doc);
    if !scripts.is_empty() {
        let res = crate::js::run(&doc, &scripts);
        doc = res.tree;
        if !res.errors.is_empty() {
            kprintln!("BROWSER: js: {} issue(s); first: {}", res.errors.len(), res.errors[0]);
        }
        let mut body_text = String::new();
        if let Some(b) = doc.find("body") {
            b.text(&mut body_text);
        }
        kprintln!("BROWSER: ran {} script(s); body text now {} chars", scripts.len(), body_text.trim().len());
    }

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

    // Web fonts: parse @font-face rules from the fetched stylesheets, fetch the
    // TTFs (Google Fonts serves plain TrueType to our generic User-Agent — no
    // WOFF2/Brotli needed) and register them with FreeType so the page renders
    // in its actual typefaces (Cormorant Garamond, Barlow Condensed, Lora).
    crate::freetype::clear_web_fonts();
    crate::glyph_cache::clear();
    register_font_faces(&all_css);

    // Images (viewport-aware): build a slot for every unique <img> src, sized
    // from its width/height attrs (or a default box). Decode only the slots near
    // the initial viewport; the rest stay placeholders and are fetched lazily on
    // scroll (lazy_load_images). This bounds memory + network on image-heavy
    // pages. Cached decodes (LRU) are reused at their real size immediately.
    let mut cache = match &mut win.app {
        crate::wm::App::Browser(st) => core::mem::take(&mut st.img_cache),
        _ => Vec::new(),
    };
    let mut img_nodes = Vec::new();
    doc.find_all("img", &mut img_nodes);
    let mut slots: Vec<ImgSlot> = Vec::new();
    let mut pixels: Vec<Option<png::Image>> = Vec::new();
    for node in img_nodes {
        let Some(src) = node.attr("src").map(resolve_href) else { continue };
        if slots.iter().any(|s| s.src == src) {
            continue;
        }
        let aw = node.attr("width").and_then(|v| v.trim().parse::<isize>().ok());
        let ah = node.attr("height").and_then(|v| v.trim().parse::<isize>().ok());
        let (bw, bh) = (aw.unwrap_or(DEFAULT_IMG_W).max(1), ah.unwrap_or(DEFAULT_IMG_H).max(1));
        // Cache hit: reuse the decoded image (and its real size) right away.
        if let Some(pos) = cache.iter().position(|(s, _)| *s == src) {
            let entry = cache.remove(pos);
            cache.insert(0, entry.clone());
            slots.push(ImgSlot { src, w: entry.1.w as isize, h: entry.1.h as isize });
            pixels.push(Some(entry.1));
        } else {
            slots.push(ImgSlot { src, w: bw, h: bh });
            pixels.push(None);
        }
    }

    // Layout at the window's content width.
    let view_w = win.cw;
    let body_node = doc.find("body").unwrap_or(&doc);
    let root = root_style();
    let body_style = resolve(&sheet, body_node, &root);
    let page_bg = body_style.bg.unwrap_or(0xffff_ffff);

    // Pass 1: lay out with placeholder/cached sizes to find each image's y, then
    // fetch the ones within ~2 viewports of the top (initial scroll = 0).
    let view_h = win.ch.saturating_sub(CHROME);
    let fetch_ahead = (view_h as isize) * 2;
    {
        let mut ctx1 = Ctx {
            sheet: &sheet, imgs: &slots, items: Vec::new(), links: Vec::new(),
            img_spots: Vec::new(), fields: Vec::new(), forms: Vec::new(), cur_form: usize::MAX,
        };
        layout_block(&mut ctx1, body_node, &root, 0, view_w as isize, 0, None);
        let mut slot_y = alloc::vec![isize::MAX; slots.len()];
        for it in &ctx1.items {
            if let &Item::Image { y, idx, .. } = it {
                if y < slot_y[idx] {
                    slot_y[idx] = y;
                }
            }
        }
        for i in 0..slots.len() {
            if pixels[i].is_none() && slot_y[i] <= fetch_ahead {
                fetch_image_slot(&mut slots, &mut pixels, i, &mut cache);
            }
        }
        let deferred = pixels.iter().filter(|p| p.is_none()).count();
        if deferred > 0 {
            kprintln!("BROWSER: {deferred}/{} image(s) deferred (off-viewport)", slots.len());
        }
    }

    // Pass 2: final layout, now with decoded sizes for the fetched images.
    let mut ctx = Ctx {
        sheet: &sheet, imgs: &slots, items: Vec::new(), links: Vec::new(),
        img_spots: Vec::new(), fields: Vec::new(), forms: Vec::new(), cur_form: usize::MAX,
    };
    let end_y = layout_block(&mut ctx, body_node, &root, 0, view_w as isize, 0, None);

    // Return the (possibly grown) cache to the window for the next navigation.
    if let crate::wm::App::Browser(st) = &mut win.app {
        st.img_cache = cache;
        st.page = Vec::new(); // free the old band buffer before reallocating
    }
    // The document is kept as a retained display list and rasterized one band at
    // a time (see repaint_band), so its full logical height — capped only at
    // MAX_DOC_H — stays scrollable even when it's 10000+ px tall.
    let doc_h = (end_y.max(1) as usize).min(MAX_DOC_H);

    // Measure the text runs (for find-in-page) and gather all visible text over
    // the whole document, independent of which band is currently rasterized.
    let mut text_runs: Vec<(isize, isize, isize, String)> = Vec::new();
    {
        let mut dummy = [0u32; 1];
        let mfb = unsafe { Framebuffer::new(dummy.as_mut_ptr(), 1, 1, 4) };
        for item in &ctx.items {
            if let Item::Text { x, y, s, font, .. } = item {
                if *y >= 0 {
                    let rw = mfb.measure_text(s, font.id, font.px).0 as isize;
                    text_runs.push((*x, *y, rw, s.to_lowercase()));
                }
            }
        }
    }

    // Proof breadcrumbs: where things ended up, in document coordinates.
    for &(idx, x, y) in &ctx.img_spots {
        let slot = &slots[idx];
        kprintln!("BROWSER: img '{}' at ({x}, {y}) {}x{}", slot.src, slot.w, slot.h);
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
    st.path = path.clone();
    // Sync the active tab's path + a short title for the tab strip.
    let a = st.active;
    if a < st.tabs.len() {
        st.tabs[a].path = path.clone();
        st.tabs[a].title = tab_title(&path);
    }
    st.items = ctx.items;
    st.img_src = slots.iter().map(|s| s.src.clone()).collect();
    st.imgs = pixels;
    st.page_w = view_w;
    st.doc_h = doc_h;
    st.band_top = 0;
    st.links = ctx.links;
    st.fields = ctx.fields;
    st.forms = ctx.forms;
    let mut text = String::new();
    for it in &st.items {
        if let Item::Text { s, .. } = it {
            text.push_str(s);
            text.push(' ');
        }
    }
    st.page_text = text;
    st.text_runs = text_runs;
    st.find_open = false;
    st.find_query.clear();
    st.find_matches.clear();
    st.find_idx = 0;
    st.focus = None;
    st.scroll = 0;
    st.page_bg = page_bg;
    let nfields = st.fields.len();
    for f in &st.fields {
        let kind = match f.kind {
            InputKind::Text => "text",
            InputKind::Password => "password",
            InputKind::Hidden => "hidden",
            InputKind::Checkbox => "checkbox",
            InputKind::Radio => "radio",
            InputKind::Submit => "submit",
            InputKind::Textarea => "textarea",
            InputKind::Select => "select",
        };
        kprintln!(
            "BROWSER: field '{kind}' name='{}' at ({}, {}) {}x{} checked={}",
            f.name, f.x, f.y, f.w, f.h, f.checked
        );
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
    st.page = Vec::new();
    st.page_w = cw;
    st.doc_h = 64;
    st.band_top = 0;
    st.imgs = Vec::new();
    st.img_src = Vec::new();
    st.items = alloc::vec![Item::Text {
        x: 8,
        y: 8,
        s: text,
        color: 0xffa0_2020,
        scale: 1,
        font: Font { id: crate::freetype::FontId::Ui, px: 15 },
    }];
    st.links = Vec::new();
    st.fields = Vec::new();
    st.forms = Vec::new();
    st.text_runs = Vec::new();
    st.scroll = 0;
    st.page_bg = 0xffff_ffff;
    repaint_band(win);
    paint_view(win);
}

// --- window plumbing -----------------------------------------------------------

/// Repaint the canvas: visible page rows below a URL bar.
pub fn paint_view(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let view_h = ch - CHROME;
    // Make sure the rasterized band covers the scroll window before blitting.
    ensure_band(win, view_h);
    let bar = {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        let bh = if st.page_w == 0 { 0 } else { st.page.len() / st.page_w };
        for row in 0..view_h {
            let sy = st.scroll + row;
            let by = sy.wrapping_sub(st.band_top); // band-relative row
            let dst = &mut win.canvas[(CHROME + row) * cw..(CHROME + row) * cw + cw];
            if sy < st.doc_h && st.page_w == cw && by < bh {
                dst.copy_from_slice(&st.page[by * cw..by * cw + cw]);
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
        let z = if st.zoom != 100 { format!("  {}%z", st.zoom) } else { String::new() };
        // While editing, show the edit buffer with a block cursor.
        if st.editing {
            format!("{}_", st.edit_buf)
        } else if st.path.starts_with("http://") || st.path.starts_with("https://") {
            format!("{}  [{pct}%]{z}", st.path)
        } else {
            format!("http://{}{}  [{pct}%]{z}", net::fmt_ip(&ip), st.path)
        }
    };
    let (doc_h, scroll, ntabs, active, can_back, can_fwd, titles) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        let a = st.active;
        (
            st.doc_h, st.scroll, st.tabs.len(), a,
            !st.tabs[a].back.is_empty(), !st.tabs[a].fwd.is_empty(),
            st.tabs.iter().map(|t| t.title.clone()).collect::<Vec<_>>(),
        )
    };
    let fb = win.canvas_fb();
    // Scrollbar.
    if doc_h > view_h {
        fb.fill_round_rect(cw - 5, CHROME + 1, 4, view_h - 2, 2, 0xff44_4444);
        let thumb_h = (view_h * view_h / doc_h).max(16).min(view_h);
        let thumb_y = CHROME + scroll * (view_h - thumb_h) / (doc_h - view_h);
        fb.fill_round_rect(cw - 5, thumb_y, 4, thumb_h, 2, 0xff88_8888);
    }
    // Tab strip (top row).
    fb.fill_rect(0, 0, cw, TABBAR_H, 0xff20_2428);
    let (cells, (nt_x, nt_w)) = tab_layout(ntabs, cw);
    for (i, &(tx, tw)) in cells.iter().enumerate() {
        let bg = if i == active { 0xffc8_ccd4 } else { 0xff34_3a40 };
        let fg = if i == active { 0xff20_2830 } else { 0xffc0_c4cc };
        fb.fill_rect(tx + 1, 2, tw.saturating_sub(2), TABBAR_H - 2, bg);
        let maxc = (tw.saturating_sub(24)) / 8;
        let title: String = titles[i].chars().take(maxc).collect();
        fb.draw_string(tx + 6, 4, &title, fg, None);
        // close 'x'
        fb.draw_string(tx + tw - 14, 4, "x", fg, None);
    }
    // New-tab '+' button.
    fb.fill_rect(nt_x + 1, 2, nt_w.saturating_sub(2), TABBAR_H - 2, 0xff34_3a40);
    fb.draw_string(nt_x + 7, 4, "+", 0xffc0_c4cc, None);

    // Address-bar row (below the tab strip): back '<', forward '>', then URL.
    fb.fill_rect(0, TABBAR_H, cw, TOPBAR, BAR_BG);
    fb.fill_rect(0, TABBAR_H, 18, TOPBAR, if can_back { 0xff90_a8c0 } else { 0xffb0_b4bc });
    fb.draw_string(5, TABBAR_H + 2, "<", BAR_TEXT, None);
    fb.fill_rect(18, TABBAR_H, 18, TOPBAR, if can_fwd { 0xff90_a8c0 } else { 0xffb0_b4bc });
    fb.draw_string(23, TABBAR_H + 2, ">", BAR_TEXT, None);
    fb.draw_string(40, TABBAR_H + 2, &bar, BAR_TEXT, None);

    // M36 find-in-page: highlight matches in view + a find bar at the bottom.
    if let crate::wm::App::Browser(st) = &win.app {
        if st.find_open {
            let view_h = ch - CHROME;
            for (mi, &ri) in st.find_matches.iter().enumerate() {
                let (rx, ry, rw, _) = &st.text_runs[ri];
                let py = *ry as usize;
                if py >= st.scroll && py + 18 < st.scroll + view_h {
                    let cy = CHROME + (py - st.scroll);
                    let (col, a) = if mi == st.find_idx { (0xffff_a000, 150) } else { (0xffff_e000, 90) };
                    fb.blend_rect((*rx).max(0) as usize, cy, (*rw).max(6) as usize, 18, col, a);
                }
            }
            let by = ch - 26;
            fb.fill_rect(0, by, cw, 26, 0xff2a_2a2a);
            let n = if st.find_matches.is_empty() { 0 } else { st.find_idx + 1 };
            let label = alloc::format!("Find: {}_    {} of {}", st.find_query, n, st.find_matches.len());
            fb.draw_text(8, by + 4, &label, crate::freetype::FontId::Ui, 14, 0xffe8_e8e8);
        }
    }
}

// --- M36 find-in-page (Ctrl+F) -------------------------------------------------

pub fn find_toggle(win: &mut Window) {
    let crate::wm::App::Browser(st) = &mut win.app else { return };
    st.find_open = !st.find_open;
    if !st.find_open {
        st.find_matches.clear();
    }
    paint_view(win);
}

fn find_recompute(win: &mut Window) {
    {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        let q = st.find_query.to_lowercase();
        st.find_matches.clear();
        if !q.is_empty() {
            for (i, (_, _, _, t)) in st.text_runs.iter().enumerate() {
                if t.contains(&q) {
                    st.find_matches.push(i);
                }
            }
        }
        st.find_idx = 0;
        scroll_to_match(st);
        crate::kprintln!("BROWSER: find '{}' -> {} matches", st.find_query, st.find_matches.len());
    }
    paint_view(win);
}

fn scroll_to_match(st: &mut BrowserState) {
    if let Some(&ri) = st.find_matches.get(st.find_idx) {
        let y = st.text_runs[ri].1.max(0) as usize;
        st.scroll = y.saturating_sub(60).min(st.doc_h.saturating_sub(1));
    }
}

/// Returns true if the browser consumed the character (find bar is open).
pub fn find_char(win: &mut Window, ch: char) -> bool {
    {
        let crate::wm::App::Browser(st) = &win.app else { return false };
        if !st.find_open {
            return false;
        }
    }
    match ch {
        '\u{1b}' => find_toggle(win), // Esc closes
        '\n' => find_advance(win, 1),
        '\u{8}' => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                st.find_query.pop();
            }
            find_recompute(win);
        }
        c if !c.is_control() => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                st.find_query.push(c);
            }
            find_recompute(win);
        }
        _ => {}
    }
    true
}

pub fn find_advance(win: &mut Window, dir: isize) {
    {
        let crate::wm::App::Browser(st) = &mut win.app else { return };
        if st.find_matches.is_empty() {
            return;
        }
        let n = st.find_matches.len() as isize;
        st.find_idx = (st.find_idx as isize + dir).rem_euclid(n) as usize;
        scroll_to_match(st);
    }
    paint_view(win);
}

pub fn find_is_open(win: &Window) -> bool {
    matches!(&win.app, crate::wm::App::Browser(st) if st.find_open)
}

/// Canvas-relative click: focus an on-page input field if one was hit.
/// Returns true if a field took focus.
pub fn focus_field(win: &mut Window, rx: isize, ry: isize) -> bool {
    let i = {
        let crate::wm::App::Browser(st) = &mut win.app else { return false };
        if (ry as usize) < CHROME {
            return false;
        }
        let (dx, dy) = (rx, ry - CHROME as isize + st.scroll as isize);
        st.fields.iter().position(|f| {
            f.kind != InputKind::Hidden && dx >= f.x && dx < f.x + f.w && dy >= f.y && dy < f.y + f.h
        })
    };
    let Some(i) = i else { return false };

    let kind = {
        let crate::wm::App::Browser(st) = &win.app else { return false };
        st.fields[i].kind
    };
    match kind {
        InputKind::Text | InputKind::Password | InputKind::Textarea => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                st.focus = Some(i);
                st.editing = false;
            }
            paint_fields(win);
        }
        InputKind::Checkbox => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                st.fields[i].checked = !st.fields[i].checked;
            }
            navigate_reflow(win);
        }
        InputKind::Radio => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                let (name, form) = (st.fields[i].name.clone(), st.fields[i].form);
                for f in st.fields.iter_mut() {
                    if f.kind == InputKind::Radio && f.form == form && f.name == name {
                        f.checked = false;
                    }
                }
                st.fields[i].checked = true;
            }
            navigate_reflow(win);
        }
        InputKind::Select => {
            if let crate::wm::App::Browser(st) = &mut win.app {
                let f = &mut st.fields[i];
                if !f.options.is_empty() {
                    let cur = f.options.iter().position(|o| *o == f.value).unwrap_or(0);
                    let next = (cur + 1) % f.options.len();
                    f.value = f.options[next].clone();
                }
            }
            navigate_reflow(win);
        }
        InputKind::Submit => {
            let form = {
                let crate::wm::App::Browser(st) = &win.app else { return true };
                st.fields[i].form
            };
            submit_form(win, form, Some(i));
        }
        InputKind::Hidden => {}
    }
    true
}

/// Repaint controls after a checkbox/radio/select state change (no re-fetch —
/// `paint_fields` redraws live field state straight into the page buffer).
fn navigate_reflow(win: &mut Window) {
    paint_fields(win);
}

/// Build and send a form's submission. `clicked` is the submit button that
/// triggered it (its name=value is included), if any.
fn submit_form(win: &mut Window, form_idx: usize, clicked: Option<usize>) {
    let (is_post, action, body) = {
        let crate::wm::App::Browser(st) = &win.app else { return };
        if form_idx >= st.forms.len() {
            return;
        }
        let form = &st.forms[form_idx];
        let mut pairs: Vec<(String, String)> = Vec::new();
        for (idx, f) in st.fields.iter().enumerate() {
            if f.form != form_idx || f.name.is_empty() {
                continue;
            }
            match f.kind {
                InputKind::Checkbox | InputKind::Radio => {
                    if f.checked {
                        pairs.push((f.name.clone(), f.value.clone()));
                    }
                }
                InputKind::Submit => {
                    if Some(idx) == clicked {
                        pairs.push((f.name.clone(), f.value.clone()));
                    }
                }
                _ => pairs.push((f.name.clone(), f.value.clone())),
            }
        }
        let body: String = pairs
            .iter()
            .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        (form.method == "POST", form.action.clone(), body)
    };
    if is_post {
        kprintln!("BROWSER: POST {action} ({} bytes)", body.len());
        navigate_body(win, &action, Some(body.into_bytes()), true);
    } else {
        let sep = if action.contains('?') { '&' } else { '?' };
        let url = if body.is_empty() { action } else { format!("{action}{sep}{body}") };
        kprintln!("BROWSER: GET-submit {url}");
        navigate(win, &url, true);
    }
}

/// Percent-encode a form field name/value (x-www-form-urlencoded).
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Canvas-relative click -> link href, if one was hit.
pub fn link_at(win: &Window, rx: isize, ry: isize) -> Option<String> {
    let crate::wm::App::Browser(st) = &win.app else { return None };
    if (ry as usize) < CHROME {
        return None;
    }
    let (dx, dy) = (rx, ry - CHROME as isize + st.scroll as isize);
    st.links
        .iter()
        .find(|l| dx >= l.x && dx < l.x + l.w && dy >= l.y && dy < l.y + l.h)
        .map(|l| l.href.clone())
}

static SCROLL_DONE: AtomicBool = AtomicBool::new(false);

/// Scroll to an absolute offset (clamped to the document), repaint, and emit
/// SCROLL_OK the first time a taller-than-window page moves off its top.
fn scroll_to(win: &mut Window, target: isize) -> bool {
    let view_h = win.ch - CHROME;
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
    // Pull in any images that just scrolled near the viewport, then paint.
    lazy_load_images(win, view_h);
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
                // Enter in a single-line text field submits its form.
                let (form, multiline) = if let crate::wm::App::Browser(st) = &mut win.app {
                    let f = st.focus.and_then(|i| st.fields.get(i));
                    let r = (f.map(|f| f.form), f.map(|f| f.multiline).unwrap_or(false));
                    if !r.1 {
                        st.focus = None;
                    }
                    r
                } else {
                    (None, false)
                };
                if multiline {
                    if let crate::wm::App::Browser(st) = &mut win.app {
                        if let Some(f) = st.focus.and_then(|i| st.fields.get_mut(i)) {
                            f.value.push('\n');
                        }
                    }
                    paint_fields(win);
                } else if let Some(fi) = form.filter(|&fi| fi != usize::MAX) {
                    submit_form(win, fi, None);
                } else {
                    paint_fields(win);
                }
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
    let half = (win.ch - CHROME) as isize / 2;
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
