//! M32-B: the Lisp REPL window (App::Lisp). A terminal-styled view over the
//! `lisp` interpreter — green-on-black, scrollable output history, a single
//! input line with a `> ` prompt, and Up/Down input-history recall. A self
//! test runs at startup and emits LISP_OK when it passes.

use crate::lisp::{self, Interp};
use crate::wm::{App, Window};
use crate::kprintln;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BG: u32 = 0xff04_140a; // near-black green tint
const FG: u32 = 0xff30_ff60; // phosphor green
const DIM: u32 = 0xff20_9040;
const ROW: usize = 16;

const KEY_UP: u16 = 103;
const KEY_PGUP: u16 = 104;
const KEY_DOWN: u16 = 108;
const KEY_PGDN: u16 = 109;

pub struct LispState {
    interp: Interp,
    output: Vec<String>, // scrollback
    input: String,
    hist: Vec<String>, // input history (max 50)
    hidx: usize,        // position in hist; == len means "current line"
    scroll: usize,      // top output line shown; usize::MAX = pinned to bottom
}

const SELF_TEST: &[(&str, &str)] = &[
    ("(+ 1 2)", "3"),
    ("(define x 10) x", "10"),
    ("(lambda (n) (* n n))", "#<lambda>"),
    ("((lambda (n) (* n n)) 5)", "25"),
    ("(define (fact n) (if (= n 0) 1 (* n (fact (- n 1))))) (fact 10)", "3628800"),
    ("(car (list 1 2 3))", "1"),
    ("(map (lambda (x) (* x x)) (list 1 2 3 4 5))", "(1 4 9 16 25)"),
];

fn run_self_test(interp: &mut Interp) {
    let mut all = true;
    for (src, want) in SELF_TEST {
        match interp.eval_str(src) {
            Ok(got) => {
                let _ = lisp::take_output();
                kprintln!("LISP: {src} => {got}");
                if got != *want {
                    all = false;
                    kprintln!("LISP MISMATCH: wanted {want}");
                }
            }
            Err(e) => {
                all = false;
                kprintln!("LISP ERROR: {src}: {e}");
            }
        }
    }
    if all {
        kprintln!("LISP_OK");
    }
}

impl LispState {
    pub fn new() -> LispState {
        let mut interp = Interp::new();
        run_self_test(&mut interp);
        let output = [
            "Veil Lisp 1.0",
            "A Lisp interpreter in a from-scratch OS.",
            "Type (help) for examples.",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        LispState {
            interp,
            output,
            input: String::new(),
            hist: Vec::new(),
            hidx: 0,
            scroll: usize::MAX,
        }
    }

    fn push_lines(&mut self, text: &str) {
        for l in text.split('\n') {
            self.output.push(l.to_string());
        }
    }

    fn eval_input(&mut self) {
        let line = core::mem::take(&mut self.input);
        self.output.push(format!("> {line}"));
        if !line.trim().is_empty() {
            if self.hist.last().map(String::as_str) != Some(line.as_str()) {
                self.hist.push(line.clone());
                if self.hist.len() > 50 {
                    self.hist.remove(0);
                }
            }
            match self.interp.eval_str(&line) {
                Ok(result) => {
                    let printed = lisp::take_output();
                    if !printed.is_empty() {
                        self.push_lines(printed.trim_end_matches('\n'));
                    }
                    self.output.push(result);
                }
                Err(e) => {
                    let _ = lisp::take_output();
                    self.output.push(format!("error: {e}"));
                }
            }
        }
        self.hidx = self.hist.len();
        self.scroll = usize::MAX; // pin to bottom
    }
}

/// Special keys with no character form: history recall + output scroll.
pub fn key(win: &mut Window, code: u16) -> bool {
    let vis = visible_rows(win.ch);
    let App::Lisp(st) = &mut win.app else { return false };
    match code {
        KEY_UP => {
            if st.hidx > 0 {
                st.hidx -= 1;
                st.input = st.hist.get(st.hidx).cloned().unwrap_or_default();
            }
        }
        KEY_DOWN => {
            if st.hidx < st.hist.len() {
                st.hidx += 1;
                st.input = st.hist.get(st.hidx).cloned().unwrap_or_default();
            }
        }
        KEY_PGUP => {
            let top = st.scroll_top(vis);
            st.scroll = top.saturating_sub(vis / 2);
        }
        KEY_PGDN => {
            let top = st.scroll_top(vis).saturating_add(vis / 2);
            st.scroll = top.min(st.output.len().saturating_sub(vis));
        }
        _ => return false,
    }
    true
}

/// A typed character: Enter evaluates, Backspace deletes, printables append.
pub fn char_input(win: &mut Window, ch: char) {
    let App::Lisp(st) = &mut win.app else { return };
    match ch {
        '\n' | '\r' => st.eval_input(),
        '\u{8}' | '\u{7f}' => {
            st.input.pop();
        }
        c if (' '..='~').contains(&c) => st.input.push(c),
        _ => {}
    }
}

fn visible_rows(ch: usize) -> usize {
    (ch / ROW).saturating_sub(1).max(1) // reserve the bottom row for input
}

impl LispState {
    fn scroll_top(&self, vis: usize) -> usize {
        if self.scroll == usize::MAX {
            self.output.len().saturating_sub(vis)
        } else {
            self.scroll.min(self.output.len().saturating_sub(vis))
        }
    }
}

pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let cols = (cw / 8).saturating_sub(1).max(1);
    let vis = visible_rows(ch);
    let (lines, input, top) = {
        let App::Lisp(st) = &win.app else { return };
        (st.output.clone(), st.input.clone(), st.scroll_top(vis))
    };
    let fb = win.canvas_fb();
    fb.clear(BG);
    for r in 0..vis {
        let Some(line) = lines.get(top + r) else { break };
        let s: String = line.chars().take(cols).collect();
        let color = if s.starts_with("> ") { DIM } else { FG };
        fb.draw_string(4, r * ROW, &s, color, None);
    }
    // Input line, pinned to the bottom row.
    let prompt = format!("> {input}");
    let shown: String = prompt.chars().rev().take(cols).collect::<Vec<_>>().into_iter().rev().collect();
    let y = (vis) * ROW;
    fb.draw_string(4, y, &shown, FG, None);
    // Block cursor.
    let cx = 4 + shown.chars().count() * 8;
    fb.fill_rect(cx, y, 8, ROW, DIM);
}
