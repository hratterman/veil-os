//! M35 WASM app window: opening a .WSM file parses the module, runs it (a
//! `_start`/`main` prints via fd_write; a `compute`/`fib` export is called and
//! JIT-compiled), and shows the output + a short module description.

use crate::wm::{App, Window};
use crate::{fs, kprintln, wasm};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff0d_0d0d;
const TEXT: u32 = 0xffe8_e8e8;
const ACCENT: u32 = 0xff5b_8af0;
const MUTED: u32 = 0xff88_8888;

pub struct WasmState {
    name: String,
    lines: Vec<(String, u32)>, // (text, colour)
}

impl WasmState {
    pub fn with_file(name: &str) -> WasmState {
        let mut lines = Vec::new();
        let data = fs::read_file(name).unwrap_or_default();
        lines.push((format!("$ run {name}"), ACCENT));
        lines.push((wasm::describe(&data), MUTED));

        // 1) A WASI-style module with _start/main: run it, show its stdout.
        let mut ran = false;
        if data.starts_with(b"\0asm") {
            match wasm::run(&data) {
                Ok(out) if !out.is_empty() => {
                    for l in out.lines() {
                        lines.push((String::from(l), TEXT));
                    }
                    ran = true;
                    kprintln!("WASMAPP: {name} _start printed {} bytes", out.len());
                }
                Ok(_) => {}
                Err(e) => {
                    // No _start — try a known compute export instead.
                    if e.contains("_start") {
                        ran = Self::run_compute(&data, &mut lines, name);
                    } else {
                        lines.push((format!("error: {e}"), 0xffd0_5a4a));
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
        WasmState { name: String::from(name), lines }
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

pub fn render(win: &mut Window) {
    let lines = {
        let App::Wasm(st) = &win.app else { return };
        st.lines.clone()
    };
    let fb = win.canvas_fb();
    fb.clear(BG);
    let mut y = 8;
    for (text, color) in &lines {
        fb.draw_string(8, y, text, *color, None);
        y += 16;
    }
}
