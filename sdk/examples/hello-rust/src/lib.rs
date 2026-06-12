//! "Hello, Veil" — the canonical Veil OS app. Draws a title, a button, and a
//! click counter; clicking the button increments the counter. Built for
//! `wasm32-unknown-unknown`; the output `.wasm` is renamed to `HELLO.WSM` and
//! opened in Veil's file manager.
//!
//!   cargo build --release --target wasm32-unknown-unknown
//!   cp target/wasm32-unknown-unknown/release/hello_veil.wasm HELLO.WSM

#![no_std]

use veil_sdk as v;
use veil_sdk::color;

// The button's hit box.
const BX: i32 = 20;
const BY: i32 = 104;
const BW: i32 = 150;
const BH: i32 = 44;

// App state lives in linear memory, which Veil preserves across frames.
static mut CLICKS: i32 = 0;

/// Called once when the app opens.
#[no_mangle]
pub extern "C" fn init() {
    unsafe { CLICKS = 0 };
}

/// Draw the UI. Called on open and after every event.
#[no_mangle]
pub extern "C" fn render() {
    v::clear(color::BG);
    v::draw_text(20, 14, "Hello, Veil!", color::ACCENT, 28);
    v::draw_text(20, 58, "A WebAssembly app built with the Veil SDK.", color::MUTED, 15);
    v::draw_text(20, 80, "Click the button below.", color::MUTED, 15);

    // Button.
    v::fill_rect(BX, BY, BW, BH, color::GREEN);
    v::draw_text(BX + 30, BY + 11, "Click me", color::WHITE, 18);

    // Counter.
    v::draw_text(20, 164, "Clicks:", color::MUTED, 18);
    let n = unsafe { CLICKS };
    v::draw_int(96, 164, n, color::GOLD, 18);
}

/// Handle a click at (x, y) in surface coordinates.
#[no_mangle]
pub extern "C" fn on_click(x: i32, y: i32) {
    if x >= BX && x < BX + BW && y >= BY && y < BY + BH {
        unsafe { CLICKS += 1 };
        v::log_int("clicks", unsafe { CLICKS });
    }
}
