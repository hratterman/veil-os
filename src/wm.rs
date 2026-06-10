//! M7: window manager. Overlapping windows with title bars, mouse-driven
//! focus / raise / drag, and a double-buffered compositor (back-to-front
//! into a back buffer, then one flip — no flicker, no tearing).
//!
//! Also hosts the per-window apps (M6 echo, M8 paint): each window owns a
//! private canvas buffer, which is what makes Paint strokes survive other
//! windows passing over them.
//!
//! Geometry constants are mirrored in scripts/drive_gui.py — keep in sync.

use crate::fb::Framebuffer;
use crate::{browser, clock, fs, keymap, kprintln, net, netdev, scheduler, snd, timer, viewer};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const BORDER: isize = 2;
pub const TITLE_H: isize = 22;
/// Bottom launcher bar (UX overhaul): always composited on top; windows
/// are clamped so their frames stay above it.
pub const TASKBAR_H: usize = 40;
const CLOSE_W: isize = 18; // rightmost title-bar pixels = close hit zone

const DESKTOP_BG: u32 = 0xff28_4858;
const TASKBAR_BG: u32 = 0xff18_2028;
const TASKBAR_BTN: u32 = 0xff30_4050;
const TASKBAR_BTN_OPEN: u32 = 0xff48_70a0;
const TASKBAR_TEXT: u32 = 0xffe8_eef4;

/// Launchable apps: (window title, button/icon label). Chat is filtered
/// out when no NIC is attached.
// Viewer is appended last so adding it doesn't shift the existing button
// indices (the proof drivers depend on edit=0..chat=5).
const LAUNCHERS: [(&str, &str); 8] = [
    ("edit", "Editor"),
    ("clock", "Clock"),
    ("browser", "Browser"),
    ("paint", "Paint"),
    ("shell", "Shell"),
    ("chat", "Chat"),
    ("viewer", "Viewer"),
    ("audio", "Audio"),
];
const ICON_COLORS: [u32; 8] = [
    0xff50_88c0, 0xffc0_8850, 0xff58_a878, 0xffb0_5878, 0xff60_6878, 0xff90_70b0, 0xff40_9088,
    0xffc0_6090,
];

fn launchers() -> Vec<(&'static str, &'static str)> {
    LAUNCHERS
        .iter()
        .copied()
        .filter(|(t, _)| *t != "chat" || netdev::available())
        .collect()
}
const FRAME_COLOR: u32 = 0xff10_1418;
const TITLE_FOCUSED: u32 = 0xff30_60c0;
const TITLE_UNFOCUSED: u32 = 0xff70_7880;
const TITLE_TEXT: u32 = 0xffff_ffff;
const ECHO_TEXT: u32 = 0xff20_2840;

// Paint app layout (canvas-relative).
pub const TOOLBAR_H: isize = 30;
pub const PALETTE: [u32; 8] = [
    0xff00_0000, 0xffe0_3030, 0xff30_c060, 0xff30_60e0,
    0xffe0_d030, 0xffd0_40d0, 0xff30_c0c0, 0xffff_ffff,
];
pub const BRUSH_RADII: [isize; 3] = [2, 4, 7];

const CURSOR: [&[u8]; 10] = [
    b"X.........",
    b"XX........",
    b"XOX.......",
    b"XOOX......",
    b"XOOOX.....",
    b"XOOOOX....",
    b"XOOOOOX...",
    b"XOOOOOOX..",
    b"XOOOOOOOX.",
    b"XXXXXXXXXX",
];

pub struct PaintState {
    pub color: usize, // palette index
    pub brush: usize, // radius index
    last: Option<(isize, isize)>,
}

pub struct EditorState {
    file: String,   // 8.3 FAT name, e.g. NOTE.TXT
    text: String,   // the buffer; insertion point is always the end
    status: String, // last action, shown in the toolbar
}

pub enum App {
    Echo { text: String },
    Static,
    Paint(PaintState),
    Shell { input: String, lines: Vec<String> },
    Browser(browser::BrowserState),
    Editor(EditorState),
    Clock(clock::ClockState),
    Chat { name: String, input: String, lines: Vec<String> },
    Viewer(viewer::ViewerState),
    Audio(AudioState),
}

pub struct AudioState {
    file: String,
    start_tick: u64,  // tick playback last started
    last_secs: u64,   // last elapsed value drawn (redraw cadence)
    was_playing: bool,
}

pub struct Window {
    pub title: String,
    pub x: isize,
    pub y: isize,
    pub cw: usize, // content size
    pub ch: usize,
    pub canvas: Vec<u32>,
    pub app: App,
}

impl Window {
    pub fn canvas_fb(&mut self) -> Framebuffer {
        unsafe { Framebuffer::new(self.canvas.as_mut_ptr(), self.cw, self.ch, self.cw * 4) }
    }

    fn frame_w(&self) -> isize {
        self.cw as isize + 2 * BORDER
    }

    fn frame_h(&self) -> isize {
        self.ch as isize + TITLE_H + 2 * BORDER
    }

    fn contains(&self, px: isize, py: isize) -> bool {
        px >= self.x && px < self.x + self.frame_w() && py >= self.y && py < self.y + self.frame_h()
    }
}

enum Hit {
    Title,
    Content(isize, isize), // canvas-relative
}

pub struct Wm {
    screen: Framebuffer,
    back: Vec<u32>,
    pub windows: Vec<Window>, // z-order; last = topmost = focused
    mx: isize,
    my: isize,
    buttons: u32,
    pend_x: isize,
    pend_y: isize,
    pend_buttons: u32,
    drag: Option<(usize, isize, isize)>, // (window index AT TOP, grab offset)
    shift: bool,
    abs_max: (u32, u32),
    pub dirty: bool,
}

impl Wm {
    pub fn new(screen: Framebuffer, abs_max: (u32, u32)) -> Wm {
        let (w, h) = (screen.width, screen.height);
        Wm {
            screen,
            back: vec![0u32; w * h],
            windows: Vec::new(),
            mx: 0,
            my: 0,
            buttons: 0,
            pend_x: 0,
            pend_y: 0,
            pend_buttons: 0,
            drag: None,
            shift: false,
            abs_max,
            dirty: true,
        }
    }

    /// Open `app`'s window at its default position, or raise it if it is
    /// already open. The default geometries are load-bearing: the proof
    /// drivers' click coordinates assume them.
    pub fn launch(&mut self, app: &str) {
        if let Some(idx) = self.windows.iter().position(|w| w.title == app) {
            self.raise(idx);
            return;
        }
        kprintln!("WM: launch '{app}'");
        match app {
            "shell" => self.add_window(
                "shell",
                40,
                430,
                420,
                280,
                App::Shell { input: String::new(), lines: Vec::new() },
            ),
            "edit" => {
                self.add_window("edit", 40, 40, 420, 300, App::Editor(EditorState::open("NOTE.TXT")));
                kprintln!("EDITOR: window open on NOTE.TXT");
            }
            "clock" => self.add_window("clock", 700, 36, 260, 260, App::Clock(clock::ClockState::new())),
            "paint" => self.add_window("paint", 480, 330, 480, 380, App::Paint(PaintState::new())),
            "browser" => {
                self.add_window("browser", 510, 30, 480, 620, App::Browser(browser::BrowserState::new()));
                let win = self.windows.last_mut().unwrap();
                browser::navigate(win, "/", false);
            }
            "chat" => {
                let name = match net::local_ip() {
                    Some([_, _, _, 1]) => "A",
                    Some([_, _, _, 2]) => "B",
                    _ => "me",
                };
                self.add_window(
                    "chat",
                    40,
                    380,
                    440,
                    300,
                    App::Chat {
                        name: String::from(name),
                        input: String::new(),
                        lines: Vec::new(),
                    },
                );
                kprintln!("CHAT: window open as '{name}' (udp broadcast :7777)");
            }
            "viewer" => {
                let st = viewer::ViewerState::new();
                let title = st.current_name();
                self.add_window(&title, 220, 80, 560, 460, App::Viewer(st));
                kprintln!("VIEWER: window open");
            }
            "audio" => {
                let st = AudioState {
                    file: String::from("TONE.WAV"),
                    start_tick: 0,
                    last_secs: u64::MAX,
                    was_playing: false,
                };
                self.add_window("audio", 360, 300, 300, 130, App::Audio(st));
                kprintln!("AUDIO: window open (TONE.WAV)");
            }
            _ => {}
        }
        self.dirty = true;
    }

    pub fn add_window(&mut self, title: &str, x: isize, y: isize, cw: usize, ch: usize, app: App) {
        let mut win = Window {
            title: String::from(title),
            x,
            y,
            cw,
            ch,
            canvas: vec![0xffff_ffff; cw * ch],
            app,
        };
        // Reserve the taskbar strip: frames may not extend over it.
        let max_y = self.screen.height as isize - TASKBAR_H as isize - win.frame_h();
        win.y = win.y.min(max_y);
        let fb = win.canvas_fb();
        match win.app {
            App::Static => {
                fb.clear(0xff90_50c0);
                fb.draw_string(8, 8, "static window content", 0xff20_1030, None);
            }
            App::Paint(_) => render_paint_toolbar(&fb, cw, 0, 1),
            App::Echo { .. } | App::Shell { .. } | App::Browser(_) | App::Editor(_)
            | App::Clock(_) | App::Chat { .. } | App::Viewer(_) | App::Audio(_) => {}
        }
        if matches!(win.app, App::Shell { .. }) {
            render_shell(&mut win);
        }
        if matches!(win.app, App::Editor(_)) {
            render_editor(&mut win);
        }
        if matches!(win.app, App::Clock(_)) {
            clock::render(&mut win, timer::ticks());
        }
        if matches!(win.app, App::Chat { .. }) {
            render_chat(&mut win);
        }
        if matches!(win.app, App::Viewer(_)) {
            viewer::render(&mut win);
        }
        if matches!(win.app, App::Audio(_)) {
            render_audio(&mut win);
        }
        self.windows.push(win);
        self.dirty = true;
    }

    // --- raw evdev event intake ---------------------------------------

    pub fn handle(&mut self, ev_type: u16, code: u16, value: u32) {
        match ev_type {
            keymap::EV_KEY => match code {
                keymap::KEY_LEFTSHIFT | keymap::KEY_RIGHTSHIFT => self.shift = value != 0,
                keymap::BTN_LEFT => {
                    self.pend_buttons = (self.pend_buttons & !1) | (value != 0) as u32;
                }
                keymap::BTN_RIGHT => {
                    self.pend_buttons = (self.pend_buttons & !2) | ((value != 0) as u32) << 1;
                }
                _ if value == 1 => self.on_key(code),
                _ => {}
            },
            keymap::EV_ABS => {
                // Scale device units to pixels, rounding to the nearest.
                let scale = |v: u64, range: usize, max: u32| {
                    ((v * (range as u64 - 1) + max as u64 / 2) / max as u64) as isize
                };
                match code {
                    keymap::ABS_X => {
                        self.pend_x = scale(value as u64, self.screen.width, self.abs_max.0)
                    }
                    keymap::ABS_Y => {
                        self.pend_y = scale(value as u64, self.screen.height, self.abs_max.1)
                    }
                    _ => {}
                }
            }
            keymap::EV_SYN => self.commit(),
            _ => {}
        }
    }

    fn on_key(&mut self, code: u16) {
        // Browser scroll keys / viewer arrow keys have no character form.
        if let Some(win) = self.windows.last_mut() {
            if matches!(win.app, App::Browser(_)) && browser::key(win, code) {
                self.dirty = true;
                return;
            }
            if matches!(win.app, App::Viewer(_)) && viewer::key(win, code) {
                self.dirty = true;
                return;
            }
        }
        let Some(ch) = keymap::translate(code, self.shift) else {
            return;
        };
        kprintln!("KEY: {:?}", ch);
        let mut command = None;
        if let Some(win) = self.windows.last_mut() {
            match win.app {
                App::Echo { .. } => {
                    echo_key(win, ch);
                    self.dirty = true;
                }
                App::Shell { .. } => {
                    command = shell_key(win, ch);
                    self.dirty = true;
                }
                App::Editor(_) => {
                    editor_key(win, ch);
                    self.dirty = true;
                }
                App::Chat { .. } => {
                    chat_key(win, ch);
                    self.dirty = true;
                }
                _ => {}
            }
        }
        if let Some(cmd) = command {
            self.shell_execute(&cmd);
        }
    }

    /// Run a shell command line: kernel built-ins, or a user binary
    /// loaded from the filesystem and run at EL0.
    fn shell_execute(&mut self, cmd: &str) {
        kprintln!("SHELL: $ {cmd}");
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let (name, args) = cmd.split_once(' ').unwrap_or((cmd, ""));
        match name {
            "help" => self.shell_append(
                "built-ins: help, paint\nfrom disk: ls, cat <f>, echo <s>, spin <n>, hello\n",
            ),
            "paint" => {
                let count = self
                    .windows
                    .iter()
                    .filter(|w| matches!(w.app, App::Paint(_)))
                    .count();
                let (x, y) = (520 + 24 * count as isize, 40 + 20 * count as isize);
                let title = format!("paint-{count}");
                self.add_window(&title, x, y, 480, 380, App::Paint(PaintState::new()));
                kprintln!("SHELL: launched '{title}' as a new window");
                self.shell_append(&format!("launched {title}\n"));
            }
            _ => {
                let mut file = String::from(name);
                file.make_ascii_uppercase();
                file.push_str(".BIN");
                match fs::read_file(&file) {
                    Some(bin) => match scheduler::spawn(&bin, name, args.trim()) {
                        Some(pid) => self.shell_append(&format!("[{pid}] {name} started\n")),
                        None => self.shell_append("spawn failed (out of memory?)\n"),
                    },
                    None => self.shell_append(&format!("{name}: not found on disk\n")),
                }
            }
        }
    }

    /// Append text (e.g. user program output) to the shell window.
    pub fn shell_append(&mut self, text: &str) {
        let Some(win) = self
            .windows
            .iter_mut()
            .find(|w| matches!(w.app, App::Shell { .. }))
        else {
            return;
        };
        {
            let App::Shell { lines, .. } = &mut win.app else { unreachable!() };
            for piece in text.split_inclusive('\n') {
                match lines.last_mut() {
                    // continue a line that didn't end in \n yet
                    Some(last) if !last.ends_with('\n') => last.push_str(piece),
                    _ => lines.push(String::from(piece)),
                }
            }
            let excess = lines.len().saturating_sub(100);
            if excess > 0 {
                lines.drain(..excess);
            }
        }
        render_shell(win);
        self.dirty = true;
    }

    /// EV_SYN: apply the accumulated move + button state transition.
    fn commit(&mut self) {
        let moved = (self.pend_x, self.pend_y) != (self.mx, self.my);
        if moved {
            self.mx = self.pend_x;
            self.my = self.pend_y;
            self.dirty = true;
            if let Some((idx, ox, oy)) = self.drag {
                let win = &mut self.windows[idx];
                win.x = self.mx - ox;
                let max_y = self.screen.height as isize - TASKBAR_H as isize - win.frame_h();
                win.y = (self.my - oy).min(max_y);
            } else if self.buttons & 1 != 0 {
                self.forward_mouse_move();
            }
        }
        let pressed = self.pend_buttons & !self.buttons;
        let released = self.buttons & !self.pend_buttons;
        self.buttons = self.pend_buttons;
        if pressed & 1 != 0 {
            self.on_left_down();
        }
        if released & 1 != 0 {
            self.on_left_up();
        }
    }

    fn hit_test(&self, px: isize, py: isize) -> Option<(usize, Hit)> {
        for idx in (0..self.windows.len()).rev() {
            let win = &self.windows[idx];
            if !win.contains(px, py) {
                continue;
            }
            let rel_y = py - win.y;
            let hit = if rel_y < BORDER + TITLE_H {
                Hit::Title
            } else {
                Hit::Content(px - win.x - BORDER, rel_y - BORDER - TITLE_H)
            };
            return Some((idx, hit));
        }
        None
    }

    fn raise(&mut self, idx: usize) -> usize {
        let top = self.windows.len() - 1;
        if idx != top {
            let win = self.windows.remove(idx);
            kprintln!("WM: focus -> '{}'", win.title);
            self.windows.push(win);
        }
        self.dirty = true;
        top
    }

    fn on_left_down(&mut self) {
        kprintln!("CLICK: left down @ ({}, {})", self.mx, self.my);
        if self.my >= self.screen.height as isize - TASKBAR_H as isize {
            self.taskbar_click(self.mx);
            return;
        }
        match self.hit_test(self.mx, self.my) {
            Some((idx, Hit::Title)) => {
                // Rightmost CLOSE_W px of the title bar = the X button.
                let win = &self.windows[idx];
                if self.mx - win.x - BORDER >= win.cw as isize - CLOSE_W {
                    let win = self.windows.remove(idx);
                    kprintln!("WM: closed '{}'", win.title);
                    self.dirty = true;
                    return;
                }
                let top = self.raise(idx);
                let win = &self.windows[top];
                self.drag = Some((top, self.mx - win.x, self.my - win.y));
            }
            Some((idx, Hit::Content(rx, ry))) => {
                let top = self.raise(idx);
                let win = &mut self.windows[top];
                match win.app {
                    App::Paint(_) => {
                        paint_mouse_down(win, rx, ry);
                        self.dirty = true;
                    }
                    App::Browser(_) => {
                        if let Some(href) = browser::link_at(win, rx, ry) {
                            kprintln!("BROWSER: clicked link -> {href}");
                            browser::navigate(win, &href, true);
                        }
                        self.dirty = true;
                    }
                    App::Editor(_) => {
                        editor_mouse_down(win, rx, ry);
                        self.dirty = true;
                    }
                    App::Clock(_) => {
                        clock::click(win, rx, ry);
                        self.dirty = true;
                    }
                    App::Audio(_) => {
                        if audio_click(win, rx, ry) {
                            scheduler::spawn_kernel("audio", snd::audio_task);
                        }
                        self.dirty = true;
                    }
                    _ => {}
                }
            }
            None => {
                // Bare desktop: maybe an icon launch.
                if let Some(app) = self.icon_at(self.mx, self.my) {
                    kprintln!("WM: icon -> '{app}'");
                    self.launch(app);
                }
            }
        }
    }

    fn taskbar_click(&mut self, px: isize) {
        let mut x = 70isize;
        for (app, _) in launchers() {
            if px >= x && px < x + 72 {
                kprintln!("WM: taskbar -> '{app}'");
                self.launch(app);
                return;
            }
            x += 78;
        }
    }

    /// Desktop icon hit test (vertical grid in the top-left corner).
    fn icon_at(&self, px: isize, py: isize) -> Option<&'static str> {
        if !(16..64).contains(&px) {
            return None;
        }
        for (i, (app, _)) in launchers().into_iter().enumerate() {
            let top = 16 + i as isize * 84;
            if py >= top && py < top + 64 {
                return Some(app);
            }
        }
        None
    }

    fn on_left_up(&mut self) {
        if let Some((idx, _, _)) = self.drag.take() {
            let win = &self.windows[idx];
            kprintln!("WM: '{}' moved to ({}, {})", win.title, win.x, win.y);
        } else if let Some(win) = self.windows.last_mut() {
            if let App::Paint(_) = win.app {
                paint_mouse_up(win);
            }
        }
    }

    fn forward_mouse_move(&mut self) {
        if let Some(win) = self.windows.last_mut() {
            if let App::Paint(_) = win.app {
                let rx = self.mx - win.x - BORDER;
                let ry = self.my - win.y - BORDER - TITLE_H;
                paint_mouse_move(win, rx, ry);
                self.dirty = true;
            }
        }
    }

    /// 100 ms clock repaint cadence, driven from the desktop loop's 50 Hz
    /// wakeup. Marks the WM dirty only when a clock face actually redrew.
    pub fn clock_tick(&mut self) {
        let now = timer::ticks();
        for win in &mut self.windows {
            if matches!(win.app, App::Clock(_)) && clock::tick(win, now) {
                self.dirty = true;
            }
            if matches!(win.app, App::Audio(_)) && audio_tick(win, now) {
                self.dirty = true;
            }
        }
    }

    // --- compositor -----------------------------------------------------

    pub fn compose(&mut self) {
        let (w, h) = (self.screen.width, self.screen.height);
        let back =
            unsafe { Framebuffer::new(self.back.as_mut_ptr(), w, h, w * 4) };
        back.clear(DESKTOP_BG);

        // Desktop icons (under the windows): top-left vertical grid.
        for (i, (_, label)) in launchers().into_iter().enumerate() {
            let top = 16 + i * 84;
            back.fill_rect(16, top, 48, 48, ICON_COLORS[i]);
            back.draw_char_scaled(32, top + 8, label.as_bytes()[0], 0xffff_ffff, 2);
            back.draw_string(40usize.saturating_sub(label.len() * 4), top + 52, label, 0xffd0_dce8, None);
        }

        let top = self.windows.len().saturating_sub(1);
        for (idx, win) in self.windows.iter().enumerate() {
            let focused = idx == top;
            // Frame (border) as one filled rect behind everything.
            if win.x + win.frame_w() > 0 && win.y + win.frame_h() > 0 {
                let fx = win.x.max(0) as usize;
                let fy = win.y.max(0) as usize;
                let fw = (win.x + win.frame_w()).min(w as isize) - fx as isize;
                let fh = (win.y + win.frame_h()).min(h as isize) - fy as isize;
                if fw > 0 && fh > 0 {
                    back.fill_rect(fx, fy, fw as usize, fh as usize, FRAME_COLOR);
                }
            }
            // Title bar + caption.
            let tx = win.x + BORDER;
            let ty = win.y + BORDER;
            if tx >= 0 && ty >= 0 {
                back.fill_rect(
                    tx as usize,
                    ty as usize,
                    win.cw,
                    TITLE_H as usize,
                    if focused { TITLE_FOCUSED } else { TITLE_UNFOCUSED },
                );
                back.draw_string(tx as usize + 6, ty as usize + 3, &win.title, TITLE_TEXT, None);
                // Close button: the rightmost CLOSE_W px of the title bar.
                back.fill_rect(
                    (tx + win.cw as isize - CLOSE_W) as usize,
                    ty as usize + 2,
                    CLOSE_W as usize - 2,
                    TITLE_H as usize - 4,
                    0xffc0_4848,
                );
                back.draw_string((tx + win.cw as isize - CLOSE_W) as usize + 4, ty as usize + 3, "x", TITLE_TEXT, None);
            }
            // Content.
            back.blit(win.x + BORDER, win.y + BORDER + TITLE_H, &win.canvas, win.cw, win.ch);
        }

        // Taskbar: always-on-top launcher strip across the bottom.
        let ty = h - TASKBAR_H;
        back.fill_rect(0, ty, w, TASKBAR_H, TASKBAR_BG);
        back.draw_string(8, ty + 12, "VEIL", 0xff80_b0e0, None);
        let mut bx = 70usize;
        for (app, label) in launchers() {
            let open = self.windows.iter().any(|w| w.title == app);
            back.fill_rect(bx, ty + 6, 72, 28, if open { TASKBAR_BTN_OPEN } else { TASKBAR_BTN });
            back.draw_string(bx + (72 - label.len() * 8) / 2, ty + 12, label, TASKBAR_TEXT, None);
            bx += 78;
        }
        // Status text sits just past the last launcher button (its x scales
        // with the number of launchers so they never overlap).
        let sx = 70 + launchers().len() * 78 + 16;
        back.draw_string(sx, ty + 12, "VEIL OS — from-scratch AArch64", 0xff60_7888, None);

        // Cursor, always on top.
        for (row, line) in CURSOR.iter().enumerate() {
            for (col, &c) in line.iter().enumerate() {
                let color = match c {
                    b'X' => 0xff00_0000,
                    b'O' => 0xffff_ffff,
                    _ => continue,
                };
                let (px, py) = (self.mx + col as isize, self.my + row as isize);
                if px >= 0 && py >= 0 {
                    back.put_pixel(px as usize, py as usize, color);
                }
            }
        }

        self.screen.copy_from(&self.back);
        self.dirty = false;
    }
}

// --- echo app (M6) ----------------------------------------------------------

fn echo_key(win: &mut Window, ch: char) {
    let App::Echo { text } = &mut win.app else { return };
    match ch {
        '\u{8}' => {
            text.pop();
        }
        c => text.push(c),
    }
    if text.len() > 400 {
        text.clear();
    }
    let cols = win.cw / 8 - 2;
    let mut wrapped = String::new();
    let mut col = 0;
    for c in text.chars() {
        if c == '\n' || col == cols {
            wrapped.push('\n');
            col = 0;
        }
        if c != '\n' {
            wrapped.push(c);
            col += 1;
        }
    }
    let fb = win.canvas_fb();
    fb.clear(0xffff_ffff);
    fb.draw_string(6, 6, &wrapped, ECHO_TEXT, None);
}

// --- shell app (M11) --------------------------------------------------------

const SHELL_BG: u32 = 0xff14_181c;
const SHELL_TEXT: u32 = 0xffd0_d8e0;
const SHELL_PROMPT: u32 = 0xff80_c080;

/// Returns the completed command line on Enter.
fn shell_key(win: &mut Window, ch: char) -> Option<String> {
    let App::Shell { input, lines } = &mut win.app else {
        return None;
    };
    let mut command = None;
    match ch {
        '\u{8}' => {
            input.pop();
        }
        '\n' => {
            let cmd = core::mem::take(input);
            lines.push(format!("> {cmd}\n"));
            command = Some(cmd);
        }
        c => input.push(c),
    }
    render_shell(win);
    command
}

fn render_shell(win: &mut Window) {
    let (input, visible) = {
        let App::Shell { input, lines } = &win.app else { return };
        let rows = (win.ch - 28) / 16;
        let skip = lines.len().saturating_sub(rows);
        (input.clone(), lines[skip..].to_vec())
    };
    let fb = win.canvas_fb();
    fb.clear(SHELL_BG);
    for (i, line) in visible.iter().enumerate() {
        fb.draw_string(6, 4 + 16 * i, line.trim_end_matches('\n'), SHELL_TEXT, None);
    }
    let prompt = format!("> {input}_");
    fb.draw_string(6, win.ch - 20, &prompt, SHELL_PROMPT, None);
}

// --- paint app (M8) ---------------------------------------------------------
// Canvas layout: toolbar strip (TOOLBAR_H tall) of 8 palette swatches,
// 3 brush sizes, and a CLR button; drawing surface below, persistent.

fn render_paint_toolbar(fb: &Framebuffer, cw: usize, color: usize, brush: usize) {
    fb.fill_rect(0, 0, cw, TOOLBAR_H as usize, 0xffc8_ccd4);
    for (i, &c) in PALETTE.iter().enumerate() {
        let x = 4 + 28 * i;
        let sel = if i == color { 0xff00_0000 } else { 0xff80_8890 };
        fb.fill_rect(x - 1, 1, 26, 26, sel);
        fb.fill_rect(x, 2, 24, 24, c);
    }
    for (i, &r) in BRUSH_RADII.iter().enumerate() {
        let x = 236 + 28 * i;
        let sel = if i == brush { 0xff00_0000 } else { 0xffe8_eef4 };
        fb.fill_rect(x, 2, 24, 24, sel);
        fb.fill_rect(x + 2, 4, 20, 20, 0xffe8_eef4);
        let r = r as usize;
        fb.fill_rect(x + 12 - r, 14 - r, 2 * r, 2 * r, 0xff40_4048);
    }
    fb.fill_rect(cw - 148, 2, 44, 24, 0xffb0_c8e0);
    fb.draw_string(cw - 142, 6, "LOD", 0xff20_4080, None);
    fb.fill_rect(cw - 100, 2, 44, 24, 0xffb0_e0b8);
    fb.draw_string(cw - 94, 6, "SAV", 0xff20_6030, None);
    fb.fill_rect(cw - 52, 2, 48, 24, 0xffe0_b0b0);
    fb.draw_string(cw - 44, 6, "CLR", 0xff80_2020, None);
}

// Canvas file format: 16-byte header (magic, w, h, reserved) + raw pixels
// of the drawing surface (everything below the toolbar).
const CANVAS_MAGIC: u32 = 0x3143_5556; // "VUC1"
const CANVAS_FILE: &str = "CANVAS.RAW";

fn paint_save(win: &Window) {
    let (cw, ch) = (win.cw, win.ch);
    let sh = ch - TOOLBAR_H as usize;
    let mut data = Vec::with_capacity(16 + cw * sh * 4);
    data.extend_from_slice(&CANVAS_MAGIC.to_le_bytes());
    data.extend_from_slice(&(cw as u32).to_le_bytes());
    data.extend_from_slice(&(sh as u32).to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    for px in &win.canvas[TOOLBAR_H as usize * cw..] {
        data.extend_from_slice(&px.to_le_bytes());
    }
    match fs::write_file(CANVAS_FILE, &data) {
        Ok(()) => kprintln!("PAINT: saved {cw}x{sh} canvas to {CANVAS_FILE}"),
        Err(()) => kprintln!("PAINT: save failed (no filesystem?)"),
    }
}

fn paint_load(win: &mut Window) {
    let Some(data) = fs::read_file(CANVAS_FILE) else {
        kprintln!("PAINT: load failed ({CANVAS_FILE} missing or no filesystem)");
        return;
    };
    let word = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
    let (cw, ch) = (win.cw, win.ch);
    let sh = ch - TOOLBAR_H as usize;
    if data.len() < 16 || word(0) != CANVAS_MAGIC {
        kprintln!("PAINT: {CANVAS_FILE} is not a canvas file");
        return;
    }
    let (fw, fh) = (word(4) as usize, word(8) as usize);
    if data.len() < 16 + fw * fh * 4 {
        kprintln!("PAINT: {CANVAS_FILE} is truncated");
        return;
    }
    for y in 0..fh.min(sh) {
        for x in 0..fw.min(cw) {
            let off = 16 + (y * fw + x) * 4;
            win.canvas[(TOOLBAR_H as usize + y) * cw + x] = word(off);
        }
    }
    kprintln!("PAINT: loaded {fw}x{fh} canvas from {CANVAS_FILE}");
}

fn paint_stamp(win: &mut Window, cx: isize, cy: isize) {
    let App::Paint(state) = &win.app else { return };
    let color = PALETTE[state.color];
    let r = BRUSH_RADII[state.brush];
    let (cw, ch) = (win.cw as isize, win.ch as isize);
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let (x, y) = (cx + dx, cy + dy);
            if x >= 0 && x < cw && y >= TOOLBAR_H && y < ch {
                win.canvas[(y * cw + x) as usize] = color;
            }
        }
    }
}

fn paint_line(win: &mut Window, x0: isize, y0: isize, x1: isize, y1: isize) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for i in 0..=steps {
        paint_stamp(win, x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps);
    }
}

fn paint_mouse_down(win: &mut Window, rx: isize, ry: isize) {
    if ry < TOOLBAR_H {
        let App::Paint(state) = &mut win.app else { return };
        if ry >= 2 && ry < 26 {
            for i in 0..PALETTE.len() {
                let x = 4 + 28 * i as isize;
                if rx >= x && rx < x + 24 {
                    state.color = i;
                    kprintln!("PAINT: color set to #{:06x}", PALETTE[i] & 0xff_ffff);
                }
            }
            for i in 0..BRUSH_RADII.len() {
                let x = 236 + 28 * i as isize;
                if rx >= x && rx < x + 24 {
                    state.brush = i;
                    kprintln!("PAINT: brush radius {}", BRUSH_RADII[i]);
                }
            }
        }
        let (color, brush) = (state.color, state.brush);
        let cw = win.cw as isize;
        if ry >= 2 && ry < 26 {
            if rx >= cw - 52 && rx < cw - 4 {
                let (cw, ch) = (win.cw, win.ch);
                for px in win.canvas[TOOLBAR_H as usize * cw..cw * ch].iter_mut() {
                    *px = 0xffff_ffff;
                }
                kprintln!("PAINT: cleared");
            } else if rx >= cw - 100 && rx < cw - 56 {
                paint_save(win);
            } else if rx >= cw - 148 && rx < cw - 104 {
                paint_load(win);
            }
        }
        let fb = win.canvas_fb();
        render_paint_toolbar(&fb, win.cw, color, brush);
        return;
    }
    paint_stamp(win, rx, ry);
    let App::Paint(state) = &mut win.app else { return };
    state.last = Some((rx, ry));
    kprintln!("PAINT: stroke start @ ({rx}, {ry})");
}

fn paint_mouse_move(win: &mut Window, rx: isize, ry: isize) {
    let App::Paint(state) = &mut win.app else { return };
    let Some((lx, ly)) = state.last else { return };
    state.last = Some((rx, ry));
    paint_line(win, lx, ly, rx, ry);
}

fn paint_mouse_up(win: &mut Window) {
    let App::Paint(state) = &mut win.app else { return };
    if state.last.take().is_some() {
        kprintln!("PAINT: stroke end");
    }
}

impl PaintState {
    pub fn new() -> PaintState {
        PaintState { color: 0, brush: 1, last: None }
    }
}

// --- chat app (M20) -----------------------------------------------------------
// Two Veil instances on one QEMU socket bridge exchange UDP datagrams on
// port 7777 (limited broadcast — no peer address configured anywhere).
// Message log on top, single-line input at the bottom; Enter sends
// "NAME: text\n" (<= 128 bytes), receipt is polled by the desktop loop
// via net::chat_take -> Wm::chat_append. CHAT_OK fires on this
// instance's first chat activity, sent or received.

const CHAT_BG: u32 = 0xfff8_f6f0;
const CHAT_TEXT: u32 = 0xff20_2830;
const CHAT_MINE: u32 = 0xff20_60a0;
const CHAT_PROMPT: u32 = 0xff60_3080;
static CHAT_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn chat_ok() {
    use core::sync::atomic::Ordering;
    if !CHAT_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("CHAT_OK");
    }
}

fn render_chat(win: &mut Window) {
    let (name, input, visible) = {
        let App::Chat { name, input, lines } = &win.app else { return };
        let rows = (win.ch - 28) / 16;
        let skip = lines.len().saturating_sub(rows);
        (name.clone(), input.clone(), lines[skip..].to_vec())
    };
    let mine = format!("{name}:");
    let fb = win.canvas_fb();
    fb.clear(CHAT_BG);
    for (i, line) in visible.iter().enumerate() {
        let color = if line.starts_with(&mine) { CHAT_MINE } else { CHAT_TEXT };
        fb.draw_string(6, 4 + 16 * i, line.trim_end_matches('\n'), color, None);
    }
    let prompt = format!("{name}> {input}_");
    fb.draw_string(6, win.ch - 20, &prompt, CHAT_PROMPT, None);
}

// --- audio player (M24) ----------------------------------------------------
// One window: filename, a Play/Stop button, and an elapsed-seconds readout.
// The button toggles snd playback (which runs on a kernel task so the ~3s
// stream doesn't block this loop).

const AUDIO_BTN_W: isize = 90;
const AUDIO_BTN_H: isize = 30;

fn audio_btn_rect(cw: usize, ch: usize) -> (usize, usize, usize, usize) {
    let x = (cw - AUDIO_BTN_W as usize) / 2;
    let y = ch - AUDIO_BTN_H as usize - 12;
    (x, y, AUDIO_BTN_W as usize, AUDIO_BTN_H as usize)
}

fn render_audio(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let (file, start) = {
        let App::Audio(st) = &win.app else { return };
        (st.file.clone(), st.start_tick)
    };
    let playing = snd::is_playing();
    let elapsed = if playing { (timer::ticks().saturating_sub(start)) / clock::HZ } else { 0 };
    let fb = win.canvas_fb();
    fb.clear(0xff1a_1e24);
    fb.draw_string(12, 14, &file, 0xffe8_eef4, None);
    let secs = format!("elapsed  {}:{:02}", elapsed / 60, elapsed % 60);
    fb.draw_string(12, 40, &secs, 0xff80_e0a0, None);
    let status = if playing { "playing" } else { "stopped" };
    fb.draw_string(cw - status.len() * 8 - 12, 40, status, 0xff90_a0b0, None);
    let (bx, by, bw, bh) = audio_btn_rect(cw, ch);
    let (label, bg) = if playing { ("STOP", 0xff80_3030) } else { ("PLAY", 0xff20_6030) };
    fb.fill_rect(bx, by, bw, bh, bg);
    fb.draw_string(bx + bw / 2 - 16, by + 7, label, 0xffff_ffff, None);
}

/// Content click: toggle Play/Stop. Returns true if a new stream should be
/// spawned (the caller starts the kernel task).
fn audio_click(win: &mut Window, rx: isize, ry: isize) -> bool {
    let (cw, ch) = (win.cw, win.ch);
    let (bx, by, bw, bh) = audio_btn_rect(cw, ch);
    let on_btn = rx >= bx as isize && rx < (bx + bw) as isize
        && ry >= by as isize && ry < (by + bh) as isize;
    let mut spawn = false;
    if on_btn {
        if snd::is_playing() {
            snd::stop();
            kprintln!("AUDIO: stop");
        } else if let App::Audio(st) = &mut win.app {
            st.start_tick = timer::ticks();
            snd::request(&st.file);
            kprintln!("AUDIO: play {}", st.file);
            spawn = true;
        }
    }
    render_audio(win);
    spawn
}

/// Per-frame: redraw while the elapsed second changes or play state flips.
fn audio_tick(win: &mut Window, now: u64) -> bool {
    let playing = snd::is_playing();
    let (start, last, was) = {
        let App::Audio(st) = &win.app else { return false };
        (st.start_tick, st.last_secs, st.was_playing)
    };
    let secs = if playing { now.saturating_sub(start) / clock::HZ } else { 0 };
    if secs == last && playing == was {
        return false;
    }
    if let App::Audio(st) = &mut win.app {
        st.last_secs = secs;
        st.was_playing = playing;
    }
    render_audio(win);
    true
}

fn chat_key(win: &mut Window, ch: char) {
    {
        let App::Chat { name, input, lines } = &mut win.app else { return };
        match ch {
            '\u{8}' => {
                input.pop();
            }
            '\n' => {
                let text = core::mem::take(input);
                if !text.trim().is_empty() {
                    let mut msg = format!("{name}: {}\n", text.trim());
                    msg.truncate(128);
                    if net::chat_send(msg.as_bytes()) {
                        kprintln!("CHAT: sent {} bytes: {}", msg.len(), msg.trim_end());
                        chat_ok();
                    } else {
                        kprintln!("CHAT: send failed (no netstack?)");
                    }
                    lines.push(msg);
                }
            }
            c => {
                if input.len() < 120 {
                    input.push(c);
                }
            }
        }
    }
    render_chat(win);
}

impl Wm {
    /// Append a received chat datagram to the chat window's log.
    pub fn chat_append(&mut self, msg: &str) {
        let Some(win) = self
            .windows
            .iter_mut()
            .find(|w| matches!(w.app, App::Chat { .. }))
        else {
            return;
        };
        {
            let App::Chat { lines, .. } = &mut win.app else { unreachable!() };
            for piece in msg.split_inclusive('\n') {
                if !piece.trim().is_empty() {
                    lines.push(String::from(piece.trim_end_matches('\n')));
                }
            }
            let excess = lines.len().saturating_sub(200);
            if excess > 0 {
                lines.drain(..excess);
            }
        }
        kprintln!("CHAT: rx {:?}", msg.trim_end());
        chat_ok();
        render_chat(win);
        self.dirty = true;
    }
}

// --- editor app (M18) ---------------------------------------------------------
// Toolbar strip (same height/style as Paint) with the filename + status on
// the left and LOD / SAV buttons on the right; white text area below.
// Insertion point is always the end of the buffer (no cursor movement in
// v1), drawn as an inverted block. Lines wrap at the window edge and clip
// at the bottom (no scrollback, per spec).

const EDITOR_TEXT: u32 = 0xff18_2028;
const EDITOR_MAX: usize = 8192;

/// Bytes from disk -> editable text: keep newlines + printable ASCII.
fn editor_decode(data: &[u8]) -> String {
    data.iter()
        .map(|&b| b as char)
        .filter(|&c| c == '\n' || (' '..='~').contains(&c))
        .collect()
}

impl EditorState {
    /// Open `file` from the FAT16 disk, creating it empty if missing.
    pub fn open(file: &str) -> EditorState {
        let (text, status) = match fs::read_file(file) {
            Some(data) => {
                kprintln!("EDITOR: opened {file} ({} bytes)", data.len());
                (editor_decode(&data), format!("{} bytes", data.len()))
            }
            None => {
                let status = match fs::write_file(file, b"") {
                    Ok(()) => "new file",
                    Err(()) => "create failed",
                };
                kprintln!("EDITOR: {file} missing -> {status}");
                (String::new(), String::from(status))
            }
        };
        EditorState { file: String::from(file), text, status }
    }
}

fn render_editor_toolbar(fb: &Framebuffer, cw: usize, file: &str, status: &str) {
    fb.fill_rect(0, 0, cw, TOOLBAR_H as usize, 0xffc8_ccd4);
    fb.draw_string(6, 6, &format!("{file}  [{status}]"), 0xff30_3840, None);
    fb.fill_rect(cw - 100, 2, 44, 24, 0xffb0_c8e0);
    fb.draw_string(cw - 94, 6, "LOD", 0xff20_4080, None);
    fb.fill_rect(cw - 52, 2, 44, 24, 0xffb0_e0b8);
    fb.draw_string(cw - 46, 6, "SAV", 0xff20_6030, None);
}

fn render_editor(win: &mut Window) {
    let (file, status, text) = {
        let App::Editor(st) = &win.app else { return };
        (st.file.clone(), st.status.clone(), st.text.clone())
    };
    let (cw, ch) = (win.cw, win.ch);
    let cols = (cw - 12) / 8;
    let rows = (ch - TOOLBAR_H as usize - 8) / 16;

    // Wrap into rows (hard newlines + soft wrap at the window edge).
    let mut lines: Vec<String> = vec![String::new()];
    for c in text.chars() {
        if c == '\n' || lines.last().unwrap().len() == cols {
            lines.push(String::new());
        }
        if c != '\n' {
            lines.last_mut().unwrap().push(c);
        }
    }

    let fb = win.canvas_fb();
    fb.fill_rect(0, TOOLBAR_H as usize, cw, ch - TOOLBAR_H as usize, 0xffff_ffff);
    render_editor_toolbar(&fb, cw, &file, &status);
    for (row, line) in lines.iter().take(rows).enumerate() {
        fb.draw_string(6, TOOLBAR_H as usize + 4 + 16 * row, line, EDITOR_TEXT, None);
    }
    // Block cursor at the insertion point (end of buffer), if visible.
    let (crow, ccol) = (lines.len() - 1, lines.last().unwrap().len());
    if crow < rows && ccol < cols {
        fb.fill_rect(6 + 8 * ccol, TOOLBAR_H as usize + 4 + 16 * crow, 8, 16, EDITOR_TEXT);
    }
}

fn editor_key(win: &mut Window, ch: char) {
    {
        let App::Editor(st) = &mut win.app else { return };
        match ch {
            '\u{8}' => {
                st.text.pop();
            }
            c if st.text.len() < EDITOR_MAX => st.text.push(c),
            _ => {}
        }
        st.status = String::from("edited");
    }
    render_editor(win);
}

fn editor_save(win: &mut Window) {
    let App::Editor(st) = &mut win.app else { return };
    match fs::write_file(&st.file, st.text.as_bytes()) {
        Ok(()) => {
            kprintln!("EDITOR: saved {} bytes to {}", st.text.len(), st.file);
            kprintln!("EDITOR_OK");
            st.status = format!("saved {} bytes", st.text.len());
        }
        Err(()) => {
            kprintln!("EDITOR: save of {} failed (no filesystem?)", st.file);
            st.status = String::from("save FAILED");
        }
    }
}

fn editor_load(win: &mut Window) {
    let App::Editor(st) = &mut win.app else { return };
    match fs::read_file(&st.file) {
        Some(data) => {
            st.text = editor_decode(&data);
            kprintln!("EDITOR: loaded {} bytes from {}", data.len(), st.file);
            kprintln!("EDITOR_OK");
            st.status = format!("loaded {} bytes", data.len());
        }
        None => {
            kprintln!("EDITOR: load of {} failed (missing or no filesystem)", st.file);
            st.status = String::from("load FAILED");
        }
    }
}

fn editor_mouse_down(win: &mut Window, rx: isize, ry: isize) {
    let cw = win.cw as isize;
    if ry >= 2 && ry < 26 {
        if rx >= cw - 52 && rx < cw - 8 {
            editor_save(win);
        } else if rx >= cw - 100 && rx < cw - 56 {
            editor_load(win);
        }
    }
    render_editor(win);
}
