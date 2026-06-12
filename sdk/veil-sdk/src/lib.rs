//! # veil-sdk
//!
//! Ergonomic Rust bindings for the **Veil OS** WebAssembly app ABI. A Veil app
//! is a `wasm32-unknown-unknown` module that exports `render()` (and optionally
//! `init()` and `on_click(x, y)`); it draws into the app window through the
//! `veil_*` host functions wrapped here.
//!
//! ```ignore
//! #![no_std]
//! use veil_sdk as v;
//! static mut N: i32 = 0;
//! #[no_mangle] pub extern "C" fn render() {
//!     v::clear(0xff14_1414);
//!     v::draw_text(20, 16, "Hello, Veil!", 0xff5b_8af0, 28);
//! }
//! #[no_mangle] pub extern "C" fn on_click(x: i32, y: i32) { unsafe { N += 1; } }
//! ```
//!
//! Build with `cargo build --release --target wasm32-unknown-unknown`, rename the
//! output to `MYAPP.WSM`, and open it in Veil's file manager.

#![no_std]
#![allow(clippy::missing_safety_doc)]

// The Veil host functions. The host dispatches by name, so the import module
// ("env", the wasm32 default) is irrelevant.
#[link(wasm_import_module = "env")]
extern "C" {
    fn veil_width() -> i32;
    fn veil_height() -> i32;
    fn veil_clear(color: u32);
    fn veil_fill_rect(x: i32, y: i32, w: i32, h: i32, color: u32);
    fn veil_draw_text(x: i32, y: i32, ptr: *const u8, len: i32, color: u32, size: i32);
    fn veil_log(ptr: *const u8, len: i32);
    fn veil_store_set(kp: *const u8, kl: i32, vp: *const u8, vl: i32);
    fn veil_store_get(kp: *const u8, kl: i32, out: *mut u8, cap: i32) -> i32;
    fn veil_http_get(up: *const u8, ul: i32, out: *mut u8, cap: i32) -> i32;
}

/// Width of the app drawing surface, in pixels.
pub fn width() -> i32 {
    unsafe { veil_width() }
}
/// Height of the app drawing surface, in pixels.
pub fn height() -> i32 {
    unsafe { veil_height() }
}
/// Fill the whole surface with `color` (0xRRGGBB; the alpha byte is ignored).
pub fn clear(color: u32) {
    unsafe { veil_clear(color) }
}
/// Fill a rectangle with `color`.
pub fn fill_rect(x: i32, y: i32, w: i32, h: i32, color: u32) {
    unsafe { veil_fill_rect(x, y, w, h, color) }
}
/// Draw anti-aliased text at (x, y) (top-left), `size` px tall.
pub fn draw_text(x: i32, y: i32, s: &str, color: u32, size: i32) {
    unsafe { veil_draw_text(x, y, s.as_ptr(), s.len() as i32, color, size) }
}
/// Draw a signed integer (no heap needed).
pub fn draw_int(x: i32, y: i32, n: i32, color: u32, size: i32) {
    let mut buf = [0u8; 12];
    let s = itoa(n, &mut buf);
    draw_text(x, y, s, color, size);
}
/// Write a line to the OS log (visible on the serial console).
pub fn log(s: &str) {
    unsafe { veil_log(s.as_ptr(), s.len() as i32) }
}
/// Log `label=n` (fully inlined; no slice helpers, for the Veil interpreter).
pub fn log_int(label: &str, n: i32) {
    let mut line = [0u8; 80];
    let mut len = 0usize;
    let lb = label.as_bytes();
    let mut i = 0;
    while i < lb.len() && len < line.len() {
        line[len] = lb[i];
        len += 1;
        i += 1;
    }
    if len < line.len() {
        line[len] = b'=';
        len += 1;
    }
    if n == 0 {
        if len < line.len() {
            line[len] = b'0';
            len += 1;
        }
    } else {
        let neg = n < 0;
        let mut m = if neg { (n as i64).unsigned_abs() as u32 } else { n as u32 };
        let mut digits = [0u8; 12];
        let mut d = 0;
        while m > 0 {
            digits[d] = b'0' + (m % 10) as u8;
            m /= 10;
            d += 1;
        }
        if neg && len < line.len() {
            line[len] = b'-';
            len += 1;
        }
        while d > 0 {
            d -= 1;
            if len < line.len() {
                line[len] = digits[d];
                len += 1;
            }
        }
    }
    unsafe { veil_log(line.as_ptr(), len as i32) }
}
/// Persist a key/value pair (survives app restarts, per the OS storage).
pub fn store_set(key: &str, val: &str) {
    unsafe { veil_store_set(key.as_ptr(), key.len() as i32, val.as_ptr(), val.len() as i32) }
}
/// Read a stored value into `out`; returns the number of bytes written.
pub fn store_get(key: &str, out: &mut [u8]) -> usize {
    let n = unsafe { veil_store_get(key.as_ptr(), key.len() as i32, out.as_mut_ptr(), out.len() as i32) };
    (n.max(0) as usize).min(out.len())
}
/// HTTP GET `url` over the OS network stack into `out`; returns the body length.
pub fn http_get(url: &str, out: &mut [u8]) -> i32 {
    unsafe { veil_http_get(url.as_ptr(), url.len() as i32, out.as_mut_ptr(), out.len() as i32) }
}

/// Format a signed i32 into `buf`, returning the populated `&str`. Kept to
/// basic integer ops (no UTF-8 validation) so it runs on the Veil interpreter.
pub fn itoa(n: i32, buf: &mut [u8; 12]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let neg = n < 0;
    let mut tmp = [0u8; 12];
    let mut i = 0;
    // Work in unsigned to handle i32::MIN without i64.
    let mut m = if neg { (n as i64).unsigned_abs() as u32 } else { n as u32 };
    while m > 0 {
        tmp[i] = b'0' + (m % 10) as u8;
        m /= 10;
        i += 1;
    }
    let mut j = 0;
    if neg {
        buf[j] = b'-';
        j += 1;
    }
    while i > 0 {
        i -= 1;
        buf[j] = tmp[i];
        j += 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..j]) }
}

/// Common ARGB colors (the alpha byte is ignored by the host).
pub mod color {
    pub const BG: u32 = 0xff14_1414;
    pub const TEXT: u32 = 0xffe8_e8e8;
    pub const MUTED: u32 = 0xffc8_c8c8;
    pub const ACCENT: u32 = 0xff5b_8af0;
    pub const GREEN: u32 = 0xff2f_9e6b;
    pub const GOLD: u32 = 0xffff_d060;
    pub const WHITE: u32 = 0xffff_ffff;
}

// A no_std panic handler so apps don't need to provide one. (Exactly one
// panic_handler is allowed in the whole module; this is it.)
#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
