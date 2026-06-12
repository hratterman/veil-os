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
    // M41 step 16: capability gating.
    perms: u32,
    perm_prompt: bool, // a first-launch permission dialog is showing
    requested: u32,    // capabilities the dialog asks to grant
}

impl WasmState {
    pub fn with_file(name: &str) -> WasmState {
        let data = fs::read_file(name).unwrap_or_default();

        // A graphical Veil app: run init + first render, keep memory + surface.
        if data.starts_with(b"\0asm") && wasm::app_has_render(&data) {
            kprintln!("WASMAPP: {name} is a graphical Veil app (render export)");
            let perms = crate::perms::for_app(name);
            // First-launch permission dialog for an untrusted app: ask for the
            // capabilities it doesn't already hold (default request: net + fs).
            let want = crate::perms::NETWORK | crate::perms::FILESYSTEM;
            let perm_prompt = !crate::perms::is_system(name) && (perms & want) != want;
            if perm_prompt {
                kprintln!("PERMS: {name} requests {} — showing permission dialog", crate::perms::list(want));
            }
            let frame = wasm::app_frame(&data, APP_W, APP_H, None, Some(("init", &[])), perms, name);
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
                perms,
                perm_prompt,
                requested: want,
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
        WasmState {
            name: String::from(name),
            lines,
            graphical: false,
            data: Vec::new(),
            mem: Vec::new(),
            surface: Vec::new(),
            perms: crate::perms::ALL,
            perm_prompt: false,
            requested: 0,
        }
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

// Permission-dialog button geometry (surface coords).
const PD_ALLOW: (isize, isize, isize, isize) = (APP_W as isize / 2 - 150, APP_H as isize / 2 + 10, 130, 34);
const PD_DENY: (isize, isize, isize, isize) = (APP_W as isize / 2 + 20, APP_H as isize / 2 + 10, 130, 34);

fn hit(b: (isize, isize, isize, isize), x: isize, y: isize) -> bool {
    x >= b.0 && x < b.0 + b.2 && y >= b.1 && y < b.1 + b.3
}

/// A content click in a graphical app: the permission dialog, or `on_click`.
pub fn click(win: &mut Window, rx: isize, ry: isize) {
    let (data, mem, prompt, name, requested) = {
        let App::Wasm(st) = &win.app else { return };
        if !st.graphical || rx < 0 || ry < 0 {
            return;
        }
        (st.data.clone(), st.mem.clone(), st.perm_prompt, st.name.clone(), st.requested)
    };
    // Permission dialog: Allow grants the requested caps and re-runs; Deny just
    // dismisses (the app keeps running with whatever it already had).
    if prompt {
        if hit(PD_ALLOW, rx, ry) {
            crate::perms::grant(&name, requested);
        } else if !hit(PD_DENY, rx, ry) {
            return; // click elsewhere: leave the dialog up
        }
        let perms = crate::perms::for_app(&name);
        let frame = wasm::app_frame(&data, APP_W, APP_H, Some(mem), None, perms, &name);
        if let App::Wasm(st) = &mut win.app {
            st.perm_prompt = false;
            st.perms = perms;
            if let Some(f) = frame {
                st.surface = f.px;
                st.mem = f.mem;
            }
        }
        render(win);
        return;
    }
    let perms = crate::perms::for_app(&name);
    let frame = wasm::app_frame(&data, APP_W, APP_H, Some(mem), Some(("on_click", &[rx as i64, ry as i64])), perms, &name);
    if let (App::Wasm(st), Some(f)) = (&mut win.app, frame) {
        st.surface = f.px;
        st.mem = f.mem;
    }
    render(win);
}

pub fn render(win: &mut Window) {
    let (graphical, surface, lines, prompt, name, requested) = {
        let App::Wasm(st) = &win.app else { return };
        (st.graphical, st.surface.clone(), st.lines.clone(), st.perm_prompt, st.name.clone(), st.requested)
    };
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    if graphical && surface.len() == APP_W * APP_H {
        let _ = (cw, ch);
        fb.blit(0, 0, &surface, APP_W, APP_H); // clipped to the canvas
        if prompt {
            use crate::freetype::FontId;
            // Dim, then a centered permission card.
            fb.blend_rect(0, 0, APP_W.min(cw), APP_H.min(ch), 0xff00_0000, 150);
            let (bx, by, bw, bh) = (APP_W / 2 - 170, APP_H / 2 - 60, 340, 140);
            fb.fill_round_rect(bx, by, bw, bh, 8, 0xff24_2832);
            fb.draw_text(bx + 16, by + 12, "Permission request", FontId::Ui, 18, 0xff5b_8af0);
            let msg = format!("\"{name}\" wants access to:");
            fb.draw_text(bx + 16, by + 40, &msg, FontId::Ui, 14, 0xffd0_d8e0);
            fb.draw_text(bx + 16, by + 60, &crate::perms::list(requested), FontId::Ui, 14, 0xffff_d060);
            fb.fill_round_rect((PD_ALLOW.0) as usize, (PD_ALLOW.1) as usize, PD_ALLOW.2 as usize, PD_ALLOW.3 as usize, 5, 0xff2f_9e6b);
            fb.draw_text((PD_ALLOW.0 + 38) as usize, (PD_ALLOW.1 + 8) as usize, "Allow", FontId::Ui, 15, 0xffffffff);
            fb.fill_round_rect((PD_DENY.0) as usize, (PD_DENY.1) as usize, PD_DENY.2 as usize, PD_DENY.3 as usize, 5, 0xff80_4040);
            fb.draw_text((PD_DENY.0 + 42) as usize, (PD_DENY.1 + 8) as usize, "Deny", FontId::Ui, 15, 0xffffffff);
        }
        return;
    }
    let mut y = 8;
    for (text, color) in &lines {
        fb.draw_string(8, y, text, *color, None);
        y += 16;
    }
}
