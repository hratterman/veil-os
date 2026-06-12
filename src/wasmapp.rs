//! WASM app window. Two kinds of module:
//!   * a plain WASI module (`_start`/`main` prints via fd_write, or a
//!     `compute`/`fib` export) — runs once, shows its output (M35);
//!   * a **graphical Veil app** (M41 step 12): exports `render()` (+ optional
//!     `init()` / `on_click(x,y)`) and draws via the `veil_*` graphics ABI. We
//!     keep its linear memory across frames, re-running `render` after each
//!     click so the app is interactive.

use crate::wm::{App, Window};
use crate::{fs, kprintln, wasm};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff0d_0d0d;
const TEXT: u32 = 0xffe8_e8e8;
const ACCENT: u32 = 0xff5b_8af0;
const MUTED: u32 = 0xff88_8888;

// Fixed drawing surface for a graphical app (fits the wasm window's content).
const APP_W: usize = 440;
const APP_H: usize = 200;

pub struct WasmState {
    name: String,
    lines: Vec<(String, u32)>, // non-graphical output
    // Graphical-app state (empty for plain modules).
    graphical: bool,
    data: Vec<u8>,
    mem: Vec<u8>,
    surface: Vec<u32>, // APP_W * APP_H, the last rendered frame
}

impl WasmState {
    pub fn with_file(name: &str) -> WasmState {
        let data = fs::read_file(name).unwrap_or_default();

        // A graphical Veil app: run init + first render, keep memory + surface.
        if data.starts_with(b"\0asm") && wasm::app_has_render(&data) {
            kprintln!("WASMAPP: {name} is a graphical Veil app (render export)");
            let frame = wasm::app_frame(&data, APP_W, APP_H, None, Some(("init", &[])));
            let (surface, mem) = match frame {
                Some(f) => (f.px, f.mem),
                None => (alloc::vec![BG; APP_W * APP_H], Vec::new()),
            };
            kprintln!("WASMAPP_OK: ran {name}");
            return WasmState {
                name: String::from(name),
                lines: Vec::new(),
                graphical: true,
                data,
                mem,
                surface,
            };
        }

        // Otherwise the M35 plain-module path.
        let mut lines = Vec::new();
        lines.push((format!("$ run {name}"), ACCENT));
        lines.push((wasm::describe(&data), MUTED));
        let mut ran = false;
        if data.starts_with(b"\0asm") {
            match wasm::run(&data) {
                Ok(out) if !out.is_empty() => {
                    for l in out.lines() {
                        lines.push((String::from(l), TEXT));
                        kprintln!("WASM_OUT: {l}");
                    }
                    ran = true;
                    kprintln!("WASMAPP: {name} _start printed {} bytes", out.len());
                }
                Ok(_) => {}
                Err(e) => {
                    if e.contains("_start") {
                        ran = Self::run_compute(&data, &mut lines, name);
                    } else {
                        // The app trapped (e.g. a sandbox violation). It is
                        // killed cleanly — only its window shows the error; the
                        // OS and every other app keep running.
                        lines.push((format!("app trapped: {e}"), 0xffd0_5a4a));
                        lines.push((String::from("the app was killed; the OS is unaffected"), MUTED));
                        kprintln!("WASM_KILLED: {name} trapped cleanly ({e}); OS and other apps unaffected");
                        ran = true;
                    }
                }
            }
            if !ran {
                Self::run_compute(&data, &mut lines, name);
            }
        } else {
            lines.push((String::from("not a WASM module"), 0xffd0_5a4a));
        }
        kprintln!("WASMAPP_OK: ran {name}");
        WasmState { name: String::from(name), lines, graphical: false, data: Vec::new(), mem: Vec::new(), surface: Vec::new() }
    }

    fn run_compute(data: &[u8], lines: &mut Vec<(String, u32)>, name: &str) -> bool {
        let mut any = false;
        if let Some((r, jitted)) = wasm::call_export_jit(data, "compute", 100_000) {
            lines.push((format!("compute(100000) = {r}  (jit={jitted})"), TEXT));
            kprintln!("WASMAPP: {name} compute = {r} jit={jitted}");
            any = true;
        }
        if let Some(r) = wasm::call_export(data, "fib", &[20]) {
            lines.push((format!("fib(20) = {r}"), TEXT));
            any = true;
        }
        any
    }

    pub fn title(&self) -> String {
        format!("wasm: {}", self.name)
    }
}

/// A content click in a graphical app: dispatch `on_click(x, y)`, re-render.
pub fn click(win: &mut Window, rx: isize, ry: isize) {
    let (data, mem) = {
        let App::Wasm(st) = &win.app else { return };
        if !st.graphical || rx < 0 || ry < 0 {
            return;
        }
        (st.data.clone(), st.mem.clone())
    };
    let frame = wasm::app_frame(&data, APP_W, APP_H, Some(mem), Some(("on_click", &[rx as i64, ry as i64])));
    if let (App::Wasm(st), Some(f)) = (&mut win.app, frame) {
        st.surface = f.px;
        st.mem = f.mem;
    }
    render(win);
}

pub fn render(win: &mut Window) {
    let (graphical, surface, lines) = {
        let App::Wasm(st) = &win.app else { return };
        (st.graphical, st.surface.clone(), st.lines.clone())
    };
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    if graphical && surface.len() == APP_W * APP_H {
        let _ = (cw, ch);
        fb.blit(0, 0, &surface, APP_W, APP_H); // clipped to the canvas
        return;
    }
    let mut y = 8;
    for (text, color) in &lines {
        fb.draw_string(8, y, text, *color, None);
        y += 16;
    }
}
