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

// Malformed input a user could type at the prompt. Each MUST surface as an
// Err (caught + shown by the REPL), never an out-of-bounds panic — a panic
// would exit the whole kernel. If any of these slips through to a panic the
// self-test never reaches LISP_OK, so this doubles as a robustness gate.
const BAD_INPUT: &[&str] = &[
    "(car)", "(cdr)", "(cons 1)", "(mod 5)", "(mod 5 0)", "(if)", "(if 1)",
    "(define)", "(define x)", "(lambda)", "(let)", "(let ((x)))", "(cond ())",
    "(car 5)", "(/ 1 0)", "(nope 1 2)", "(eq? 1)", "(map car)",
    "(+ 1 'a)", "(((", "(define () 1)",
];

// Self-tests run on a throwaway interp so their `define`s don't leak into the
// user's REPL env (which would then get serialized to LISP.TXT).
fn run_self_test() {
    let mut throwaway = Interp::new();
    let interp = &mut throwaway;
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
    // Robustness: malformed input must error gracefully, not panic.
    let mut bad_ok = 0;
    for src in BAD_INPUT {
        let _ = lisp::take_output();
        if interp.eval_str(src).is_err() {
            bad_ok += 1;
        } else {
            kprintln!("LISP ROBUST FAIL: {src} did not error");
        }
    }
    let _ = lisp::take_output();
    kprintln!("LISP: {bad_ok}/{} malformed inputs errored cleanly", BAD_INPUT.len());
    // Runaway non-tail recursion must hit the depth guard (Err), not overflow
    // the kernel stack. If this crashes, LISP_OK below never prints.
    let _ = lisp::take_output();
    match interp.eval_str("(define (loop n) (+ 1 (loop n))) (loop 0)") {
        Err(e) => kprintln!("LISP: deep recursion guarded ({e})"),
        Ok(_) => { all = false; kprintln!("LISP ROBUST FAIL: deep recursion not guarded"); }
    }
    if all {
        kprintln!("LISP_OK");
    }
    run_persist_test();
}

// Prove the env serialize -> restore round-trip in memory (no disk side
// effects): atoms, quoted lists and lambdas must survive. The real on-disk
// persistence (LISP.TXT) is the same machinery, exercised by the GUI driver's
// define -> close -> reopen cycle.
fn run_persist_test() {
    let mut a = Interp::new();
    let _ = a.eval_str("(define pvar 42) (define plist '(1 2 3)) (define psq (lambda (n) (* n n)))");
    let dump = a.serialize_env();
    let mut b = Interp::new();
    for line in dump.lines() {
        let t = line.trim();
        if !t.is_empty() && !t.starts_with(';') {
            let _ = b.eval_str(t);
        }
    }
    let v = b.eval_str("pvar");
    let l = b.eval_str("plist");
    let q = b.eval_str("(psq 9)");
    let _ = lisp::take_output();
    if v.as_deref() == Ok("42") && l.as_deref() == Ok("(1 2 3)") && q.as_deref() == Ok("81") {
        kprintln!("LISP_PERSIST_OK");
    } else {
        kprintln!("LISP PERSIST FAIL: pvar={v:?} plist={l:?} psq={q:?}");
    }
}

impl LispState {
    pub fn new() -> LispState {
        run_self_test();
        // Real REPL interp: builtins, then restore the user's saved env.
        let mut interp = Interp::new();
        let restored = interp.load_from("LISP.TXT");
        if restored > 0 {
            kprintln!("LISP: restored {restored} defs from LISP.TXT");
        }
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
                    // Serial echo so a GUI driver can observe REPL results
                    // (e.g. to confirm a restored value after reopen).
                    kprintln!("LISP_EVAL: {line} => {result}");
                    self.output.push(result);
                    // Persist the top-level env after anything that may have
                    // (re)bound a name. LISP.TXT is small; the write is cheap.
                    if line.contains("define") {
                        self.interp.save_to("LISP.TXT");
                    }
                }
                Err(e) => {
                    let _ = lisp::take_output();
                    kprintln!("LISP_EVAL: {line} => error: {e}");
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
