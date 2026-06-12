//! Veil Calculator — a third-party app built entirely with the Veil SDK on a
//! normal machine, compiled to `wasm32-unknown-unknown`, packaged as a `.veil`,
//! and installed with `pkg install veil-calc`. It is a *real* calculator: a
//! 4×4 button grid feeds an expression string that a from-scratch recursive-
//! descent parser evaluates with correct `+ - * /` precedence. The last result
//! is persisted via the SDK's key/value store, so it survives reopening.
//!
//! Build:
//!   rustup run 1.96.0 cargo build --release --target wasm32-unknown-unknown
//!   cp target/wasm32-unknown-unknown/release/veil_calc.wasm main.wasm
//!   python3 scripts/veil_pkg.py pack --manifest manifest.toml --wasm main.wasm --out veil-calc.veil

#![no_std]

use veil_sdk as v;
use veil_sdk::color;

// ---- expression buffer (lives in linear memory across frames) --------------
const CAP: usize = 64;
static mut EXPR: [u8; CAP] = [0; CAP];
static mut LEN: usize = 0;
static mut RESULT: i32 = 0;
static mut SHOW_RESULT: bool = false;

fn expr() -> &'static [u8] {
    unsafe { &EXPR[..LEN] }
}

fn push(c: u8) {
    unsafe {
        if SHOW_RESULT {
            LEN = 0;
            SHOW_RESULT = false;
        }
        if LEN < CAP {
            EXPR[LEN] = c;
            LEN += 1;
        }
    }
}

fn clear_expr() {
    unsafe {
        LEN = 0;
        SHOW_RESULT = false;
    }
}

// ---- recursive-descent expression evaluator --------------------------------
// Grammar:  expr := term (('+'|'-') term)*    term := factor (('*'|'/') factor)*
//           factor := number | '(' expr ')'
struct P<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> P<'a> {
    fn peek(&self) -> u8 {
        if self.i < self.b.len() { self.b[self.i] } else { 0 }
    }
    fn expr(&mut self) -> i64 {
        let mut v = self.term();
        loop {
            match self.peek() {
                b'+' => { self.i += 1; v += self.term(); }
                b'-' => { self.i += 1; v -= self.term(); }
                _ => break,
            }
        }
        v
    }
    fn term(&mut self) -> i64 {
        let mut v = self.factor();
        loop {
            match self.peek() {
                b'*' => { self.i += 1; v *= self.factor(); }
                b'/' => {
                    self.i += 1;
                    let d = self.factor();
                    v = if d != 0 { v / d } else { 0 };
                }
                _ => break,
            }
        }
        v
    }
    fn factor(&mut self) -> i64 {
        if self.peek() == b'(' {
            self.i += 1;
            let v = self.expr();
            if self.peek() == b')' { self.i += 1; }
            return v;
        }
        if self.peek() == b'-' {
            self.i += 1;
            return -self.factor();
        }
        let mut n: i64 = 0;
        let mut any = false;
        while self.peek().is_ascii_digit() {
            n = n * 10 + (self.peek() - b'0') as i64;
            self.i += 1;
            any = true;
        }
        if !any { 0 } else { n }
    }
}

fn evaluate() {
    let mut p = P { b: expr(), i: 0 };
    let r = p.expr() as i32;
    unsafe {
        RESULT = r;
        SHOW_RESULT = true;
    }
    // persist the last result
    let mut buf = [0u8; 12];
    let s = v::itoa(r, &mut buf);
    v::store_set("calc_last", s);
}

// ---- button grid -----------------------------------------------------------
const GX: i32 = 16;
const GY: i32 = 84;
const BW: i32 = 64;
const BH: i32 = 52;
const GAP: i32 = 8;

// 4×4 grid of labels (row-major).
const KEYS: [[&str; 4]; 4] = [
    ["7", "8", "9", "/"],
    ["4", "5", "6", "*"],
    ["1", "2", "3", "-"],
    ["C", "0", "=", "+"],
];

fn key_rect(col: i32, row: i32) -> (i32, i32, i32, i32) {
    (GX + col * (BW + GAP), GY + row * (BH + GAP), BW, BH)
}

#[no_mangle]
pub extern "C" fn init() {
    clear_expr();
    // restore the last result for display continuity
    let mut buf = [0u8; 12];
    let n = v::store_get("calc_last", &mut buf);
    if n > 0 {
        let mut r: i32 = 0;
        let mut neg = false;
        for (k, &c) in buf[..n].iter().enumerate() {
            if k == 0 && c == b'-' { neg = true; continue; }
            if c.is_ascii_digit() { r = r * 10 + (c - b'0') as i32; }
        }
        unsafe {
            RESULT = if neg { -r } else { r };
            SHOW_RESULT = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn render() {
    v::clear(color::BG);
    v::draw_text(16, 12, "Calculator", color::ACCENT, 22);

    // Display: the current expression, or the last result.
    v::fill_rect(16, 44, 4 * BW + 3 * GAP, 30, 0xff20_2024);
    let shown = unsafe { SHOW_RESULT };
    if shown {
        let mut buf = [0u8; 12];
        let s = v::itoa(unsafe { RESULT }, &mut buf);
        v::draw_text(24, 50, s, color::GOLD, 20);
    } else if unsafe { LEN } == 0 {
        v::draw_text(24, 50, "0", color::MUTED, 20);
    } else {
        // draw the expression bytes as text
        let e = expr();
        if let Ok(s) = core::str::from_utf8(e) {
            v::draw_text(24, 50, s, color::TEXT, 18);
        }
    }

    // Button grid.
    for (row, line) in KEYS.iter().enumerate() {
        for (col, label) in line.iter().enumerate() {
            let (x, y, w, h) = key_rect(col as i32, row as i32);
            let bg = match *label {
                "=" => color::GREEN,
                "C" => 0xffb0_3a3a,
                "+" | "-" | "*" | "/" => 0xff3a_4a6a,
                _ => 0xff2a_2a2e,
            };
            v::fill_rect(x, y, w, h, bg);
            v::draw_text(x + w / 2 - 6, y + h / 2 - 10, label, color::WHITE, 22);
        }
    }
}

#[no_mangle]
pub extern "C" fn on_click(x: i32, y: i32) {
    for (row, line) in KEYS.iter().enumerate() {
        for (col, label) in line.iter().enumerate() {
            let (bx, by, w, h) = key_rect(col as i32, row as i32);
            if x >= bx && x < bx + w && y >= by && y < by + h {
                match *label {
                    "C" => clear_expr(),
                    "=" => evaluate(),
                    s => push(s.as_bytes()[0]),
                }
                return;
            }
        }
    }
}

/// Exported so the host can drive the calculator headlessly for testing:
/// feed an ASCII expression (one byte per call via on_click of synthetic keys
/// is awkward, so this evaluates the buffer directly). Returns the result.
#[no_mangle]
pub extern "C" fn eval_ascii(c: i32) -> i32 {
    // c: a single ASCII byte to append, or 0x3D ('=') to evaluate, 0x43 ('C') clear.
    let b = c as u8;
    match b {
        b'=' => { evaluate(); unsafe { RESULT } }
        b'C' => { clear_expr(); 0 }
        _ => { push(b); unsafe { RESULT } }
    }
}
