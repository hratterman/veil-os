//! M36 calculator: + - * / % ^ sqrt, memory (M+/MR/MC), history of last 10.
//! Keyboard and click input.

use crate::freetype::FontId;
use crate::wm::{App, Window};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct CalcState {
    entry: String,    // current number being typed
    acc: f64,         // accumulator
    op: Option<char>, // pending operator
    fresh: bool,      // next digit replaces the entry
    mem: f64,
    history: Vec<String>,
}

impl CalcState {
    pub fn new() -> CalcState {
        CalcState { entry: String::from("0"), acc: 0.0, op: None, fresh: true, mem: 0.0, history: Vec::new() }
    }
}

const BG: u32 = 0xff1a_1a1a;
const DISP_BG: u32 = 0xff10_1010;
const KEY_BG: u32 = 0xff2c_2c2c;
const OP_BG: u32 = 0xff3a_3f5a;
const ACC_BG: u32 = 0xff5b_8af0;

// 5 rows x 4 cols. '_' is blank.
const KEYS: [[&str; 4]; 5] = [
    ["MC", "MR", "M+", "C"],
    ["7", "8", "9", "/"],
    ["4", "5", "6", "*"],
    ["1", "2", "3", "-"],
    ["0", ".", "=", "+"],
];
// Extra row for the less-common ops.
const KEYS2: [&str; 4] = ["sqrt", "%", "^", "<"];

fn fsqrt(x: f64) -> f64 {
    let r: f64;
    unsafe { core::arch::asm!("fsqrt {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    r
}

fn fpow(base: f64, exp: f64) -> f64 {
    // integer exponent only (sufficient for a calculator)
    let n = exp as i64;
    let mut r = 1.0f64;
    let b = if n < 0 { 1.0 / base } else { base };
    for _ in 0..n.abs() {
        r *= b;
    }
    r
}

fn fmt(v: f64) -> String {
    if v == (v as i64) as f64 && v.abs() < 1e15 {
        (v as i64).to_string()
    } else {
        format!("{v:.6}")
    }
}

fn cur(st: &CalcState) -> f64 {
    st.entry.parse().unwrap_or(0.0)
}

fn apply(st: &mut CalcState) {
    let x = cur(st);
    let r = match st.op {
        Some('+') => st.acc + x,
        Some('-') => st.acc - x,
        Some('*') => st.acc * x,
        Some('/') => {
            if x == 0.0 {
                f64::from_bits(0x7ff0_0000_0000_0000)
            } else {
                st.acc / x
            }
        }
        Some('%') => st.acc % x,
        Some('^') => fpow(st.acc, x),
        _ => x,
    };
    st.acc = r;
    st.entry = fmt(r);
    st.fresh = true;
}

/// Handle one button label (also the keyboard path maps to these).
pub fn press(win: &mut Window, label: &str) {
    let App::Calc(st) = &mut win.app else { return };
    match label {
        d if d.len() == 1 && d.chars().next().unwrap().is_ascii_digit() => {
            if st.fresh {
                st.entry.clear();
                st.fresh = false;
            }
            if st.entry == "0" {
                st.entry.clear();
            }
            st.entry.push_str(d);
        }
        "." => {
            if st.fresh {
                st.entry = String::from("0");
                st.fresh = false;
            }
            if !st.entry.contains('.') {
                st.entry.push('.');
            }
        }
        "C" => *st = CalcState::new(),
        "<" => {
            st.entry.pop();
            if st.entry.is_empty() {
                st.entry.push('0');
            }
        }
        "sqrt" => {
            st.entry = fmt(fsqrt(cur(st)));
            st.fresh = true;
        }
        "=" => {
            let lhs = fmt(st.acc);
            let opc = st.op.unwrap_or('=');
            let rhs = st.entry.clone();
            apply(st);
            let expr = format!("{lhs} {opc} {rhs} = {}", st.entry);
            crate::kprintln!("CALC: {expr}");
            st.history.push(expr);
            if st.history.len() > 10 {
                st.history.remove(0);
            }
            st.op = None;
        }
        "MC" => st.mem = 0.0,
        "MR" => {
            st.entry = fmt(st.mem);
            st.fresh = true;
        }
        "M+" => st.mem += cur(st),
        op if matches!(op, "+" | "-" | "*" | "/" | "%" | "^") => {
            if st.op.is_some() && !st.fresh {
                apply(st);
            } else {
                st.acc = cur(st);
            }
            st.op = Some(op.chars().next().unwrap());
            st.fresh = true;
        }
        _ => {}
    }
    render(win);
}

pub fn key(win: &mut Window, ch: char) {
    let label = match ch {
        '0'..='9' => {
            let mut s = String::new();
            s.push(ch);
            return press(win, &s);
        }
        '.' => ".",
        '+' => "+",
        '-' => "-",
        '*' => "*",
        '/' => "/",
        '%' => "%",
        '^' => "^",
        '\n' | '=' => "=",
        '\u{8}' => "<",
        'c' | 'C' => "C",
        _ => return,
    };
    press(win, label);
}

/// Map a content click to a key label.
pub fn click(win: &mut Window, rx: isize, ry: isize) -> bool {
    let cw = win.cw as isize;
    let disp_h = 64isize;
    let grid_top = disp_h + 4;
    let rows = 6; // 5 + the extra ops row
    let cell_h = (win.ch as isize - grid_top) / rows;
    let cell_w = cw / 4;
    if ry < grid_top {
        return false;
    }
    let r = ((ry - grid_top) / cell_h) as usize;
    let c = ((rx) / cell_w) as usize;
    if c >= 4 {
        return false;
    }
    let label = if r == 0 {
        KEYS2[c]
    } else if r <= 5 {
        KEYS[r - 1][c]
    } else {
        return false;
    };
    press(win, label);
    true
}

pub fn render(win: &mut Window) {
    let App::Calc(st) = &win.app else { return };
    let (entry, history, mem) = (st.entry.clone(), st.history.clone(), st.mem);
    let cw = win.cw;
    let ch = win.ch;
    let fb = win.canvas_fb();
    fb.clear(BG);
    // Display.
    fb.fill_round_rect(8, 8, cw - 16, 48, 6, DISP_BG);
    let (tw, _) = fb.measure_text(&entry, FontId::Mono, 28);
    fb.draw_text(cw.saturating_sub(tw + 18), 18, &entry, FontId::Mono, 28, 0xfff0f0f0);
    if mem != 0.0 {
        fb.draw_text(14, 36, "M", FontId::Ui, 12, ACC_BG);
    }
    // Keypad.
    let disp_h = 64isize;
    let grid_top = disp_h + 4;
    let rows = 6isize;
    let cell_h = (ch as isize - grid_top) / rows;
    let cell_w = cw as isize / 4;
    let draw_key = |fb: &crate::fb::Framebuffer, r: isize, c: isize, label: &str, bg: u32| {
        if label.is_empty() {
            return;
        }
        let x = c * cell_w + 3;
        let y = grid_top + r * cell_h + 3;
        fb.fill_round_rect(x as usize, y as usize, (cell_w - 6) as usize, (cell_h - 6) as usize, 6, bg);
        let (lw, _) = fb.measure_text(label, FontId::Ui, 17);
        fb.draw_text(
            (x + cell_w / 2 - lw as isize / 2) as usize,
            (y + cell_h / 2 - 11) as usize,
            label,
            FontId::Ui,
            17,
            0xffe8e8e8,
        );
    };
    for (c, label) in KEYS2.iter().enumerate() {
        draw_key(&fb, 0, c as isize, label, OP_BG);
    }
    for (r, row) in KEYS.iter().enumerate() {
        for (c, label) in row.iter().enumerate() {
            let is_op = matches!(*label, "/" | "*" | "-" | "+");
            let is_eq = *label == "=";
            let bg = if is_eq {
                ACC_BG
            } else if is_op {
                OP_BG
            } else if label.starts_with('M') || *label == "C" {
                0xff33_2c2c
            } else {
                KEY_BG
            };
            draw_key(&fb, r as isize + 1, c as isize, label, bg);
        }
    }
    let _ = history;
}
