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
use crate::{
    breakout, browser, clipboard, clock, files, font, fs, gifplayer, keymap, kprintln, net, netdev,
    repl, scheduler, shell, snake, snd, timer, video, viewer, wasmapp,
};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const BORDER: isize = 2;
pub const TITLE_H: isize = 22;
/// Bottom launcher bar (UX overhaul): always composited on top; windows
/// are clamped so their frames stay above it.
pub const TASKBAR_H: usize = 32;
const CLOSE_W: isize = 18; // rightmost title-bar pixels = close hit zone

// Desktop icon grid (shared by compose(), hit-testing, and drag drop-targets).
const ICON_W: isize = 48;
const ICON_SLOT: isize = 68; // 48 icon + 12 label + 8 gap
const ICON_COL0_X: isize = 8;
const ICON_COL1_X: isize = 76;
const ICON_TOP: isize = 8;
const ICON_HOLD_TICKS: u64 = 10; // 200 ms at the desktop's 50 Hz tick
const ICONS_FILE: &str = "ICONS.TXT";

// M35 modern dark palette.
const ACCENT: u32 = 0xff5b_8af0; // soft blue
const SURFACE: u32 = 0xff1a_1a1a; // window/title surface
const MUTED: u32 = 0xff88_8888;
const DESKTOP_BG: u32 = 0xff0d_0d0d; // near-black
const DESKTOP_GRID: u32 = 0xff14_1414; // subtle grid lines on the desktop
const TASKBAR_BG: u32 = 0xff12_1212;
const TASKBAR_BTN: u32 = 0xff1e_1e1e;
const TASKBAR_BTN_OPEN: u32 = 0xff2b_3a5c; // accent-tinted pill for the active app
const TASKBAR_TEXT: u32 = 0xffe8_e8e8;

/// Launchable apps: (window title, button/icon label). Chat is filtered
/// out when no NIC is attached.
// Viewer is appended last so adding it doesn't shift the existing button
// indices (the proof drivers depend on edit=0..chat=5).
const LAUNCHERS: [(&str, &str); 13] = [
    ("edit", "Editor"),
    ("clock", "Clock"),
    ("browser", "Browser"),
    ("paint", "Paint"),
    ("shell", "Shell"),
    ("chat", "Chat"),
    ("viewer", "Viewer"),
    ("audio", "Audio"),
    ("files", "Files"),
    ("gif", "GIF"),
    ("lisp", "Lisp"),
    ("snake", "Snake"),
    ("breakout", "Brick"),
];
// Per-app icon colours (modern, muted): browser blue, shell green, files amber,
// lisp purple, games teal/orange, etc. Indexed by LAUNCHERS order.
const ICON_COLORS: [u32; 13] = [
    0xff4a62a0, // edit  - slate blue
    0xff3f99b0, // clock - cyan
    0xff5b8af0, // browser - blue
    0xffc85a9a, // paint - pink
    0xff4f9e6a, // shell - green
    0xff4aa8a0, // chat - teal
    0xff7a6ad0, // viewer - indigo
    0xffd88a44, // audio - orange
    0xffd6a844, // files - amber
    0xffd05a4a, // gif - red
    0xff9a6ad6, // lisp - purple
    0xff45a87a, // snake - emerald
    0xffe07a44, // breakout - orange
];

fn launchers() -> Vec<(&'static str, &'static str)> {
    LAUNCHERS
        .iter()
        .copied()
        .filter(|(t, _)| *t != "chat" || netdev::available())
        .collect()
}

/// The `&'static` app name for `name`, if it is a real launcher.
fn launcher_name(name: &str) -> Option<&'static str> {
    LAUNCHERS.iter().find(|(a, _)| *a == name).map(|(a, _)| *a)
}

fn icon_label(app: &str) -> &'static str {
    LAUNCHERS.iter().find(|(a, _)| *a == app).map(|(_, l)| *l).unwrap_or("?")
}

/// Two-letter glyph for a desktop icon.
fn icon_abbrev(app: &str) -> &'static str {
    match app {
        "edit" => "Ed",
        "clock" => "Cl",
        "browser" => "Br",
        "paint" => "Pt",
        "shell" => "Sh",
        "chat" => "Ch",
        "viewer" => "Vw",
        "audio" => "Au",
        "files" => "Fl",
        "gif" => "Gf",
        "lisp" => "Li",
        "snake" => "Sn",
        "breakout" => "Bk",
        _ => "Ap",
    }
}

/// Draw a Barlow-Condensed UI string; returns its pixel width.
fn ui_text(fb: &Framebuffer, x: usize, y: usize, s: &str, font: &font::BitmapFont, color: u32) -> usize {
    fb.draw_bm_string(x, y, s, font, color);
    font::text_width(font, s)
}

/// Draw a Barlow string centred on `cx`.
fn ui_centered(fb: &Framebuffer, cx: usize, y: usize, s: &str, font: &font::BitmapFont, color: u32) {
    let w = font::text_width(font, s);
    fb.draw_bm_string(cx.saturating_sub(w / 2), y, s, font, color);
}

/// Blend colour `a` toward `b` by `t`/255.
fn blend(a: u32, b: u32, t: u32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh) & 0xff, (b >> sh) & 0xff);
        (x * (255 - t) + y * t) / 255
    };
    0xff00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// Taskbar clock — local HH:MM (or uptime before NTP sync).
fn clock_string() -> String {
    let secs = timer::wall_ticks50().map(|t| t / 50).unwrap_or_else(timer::uptime_secs);
    format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
}

/// An app keeps its colour when reordered (colour follows the app, not the
/// slot), so look it up by the app's original index in LAUNCHERS.
fn icon_color(app: &str) -> u32 {
    LAUNCHERS
        .iter()
        .position(|(a, _)| *a == app)
        .map(|i| ICON_COLORS[i])
        .unwrap_or(0xff808080)
}

/// Top-left pixel of the icon at display slot `i`, given the column-0 count.
fn icon_slot_xy(i: usize, col0_count: usize) -> (isize, isize) {
    let (col_x, row) = if i < col0_count {
        (ICON_COL0_X, i as isize)
    } else {
        (ICON_COL1_X, (i - col0_count) as isize)
    };
    (col_x, ICON_TOP + row * ICON_SLOT)
}

/// Build the desktop icon order: the saved order from ICONS.TXT (filtered to
/// apps that actually exist in this boot's launcher set), then any launchers
/// not in the saved order appended in default order. Absent/empty file = the
/// plain default order.
fn load_icon_order() -> Vec<&'static str> {
    let valid: Vec<&'static str> = launchers().iter().map(|(a, _)| *a).collect();
    let mut order: Vec<&'static str> = Vec::new();
    if crate::fs::mounted() {
        if let Some(bytes) = crate::fs::read_file(ICONS_FILE) {
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                let name = line.trim();
                if let Some(app) = launcher_name(name) {
                    if valid.contains(&app) && !order.contains(&app) {
                        order.push(app);
                    }
                }
            }
        }
    }
    for app in valid {
        if !order.contains(&app) {
            order.push(app);
        }
    }
    // Log the resolved order so a reboot test can confirm persistence.
    let mut joined = String::new();
    for (i, a) in order.iter().enumerate() {
        if i > 0 {
            joined.push(' ');
        }
        joined.push_str(a);
    }
    kprintln!("ICONS: order = {joined}");
    order
}
const FRAME_COLOR: u32 = 0xff2a_2a2a; // thin muted border
const FRAME_FOCUSED: u32 = ACCENT; // focused windows get an accent border
const TITLE_FOCUSED: u32 = SURFACE; // dark title bar, not a chunky blue one
const TITLE_UNFOCUSED: u32 = 0xff14_1414;
const TITLE_TEXT: u32 = 0xffe8_e8e8;
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
    Shell { input: String, lines: Vec<String>, history: Vec<String>, hist: usize },
    Browser(browser::BrowserState),
    Editor(EditorState),
    Clock(clock::ClockState),
    Chat(ChatState),
    Viewer(viewer::ViewerState),
    Audio(AudioState),
    Files(files::FilesState),
    Gif(gifplayer::GifPlayerState),
    Lisp(repl::LispState),
    Snake(snake::SnakeState),
    Video(video::VideoState),
    Wasm(wasmapp::WasmState),
    Breakout(breakout::BreakoutState),
}

/// One rendered chat log entry: the display text and its ink colour
/// (public-mine / public-other / DM / system).
pub struct ChatLine {
    text: String,
    color: u32,
}

/// How the Chat app reaches the network. Relay mode (M26) speaks the
/// TCP HELLO/JOIN/PART/MSG protocol and supports DMs + an online roster;
/// Udp mode is the M20 limited-broadcast fallback used when no relay is
/// configured (the two-instance LAN proof).
pub enum ChatMode {
    Udp,
    Relay { handle: net::Handle, rx: Vec<u8> },
}

pub struct ChatState {
    name: String,
    input: String,
    lines: Vec<ChatLine>,
    users: Vec<String>,          // online roster (relay mode)
    dm_target: Option<String>,   // Some -> DM compose to this user
    mode: ChatMode,
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
    ctrl: bool, // Ctrl held (clipboard shortcuts)
    alt: bool,  // Alt held (Alt+Tab task switch)
    abs_max: (u32, u32),
    pub dirty: bool,
    icon_order: Vec<&'static str>,     // desktop icon display order
    icon_press: Option<(usize, u64)>,  // (order slot, press tick) — pending tap/hold
    icon_drag: Option<usize>,          // order slot currently being dragged
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
            ctrl: false,
            alt: false,
            abs_max,
            dirty: true,
            icon_order: load_icon_order(),
            icon_press: None,
            icon_drag: None,
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
                App::Shell { input: String::new(), lines: Vec::new(), history: Vec::new(), hist: 0 },
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
                let name = chat_username();
                let (mode, users) = match net::relay_addr() {
                    Some((ip, port)) => match net::tcp_connect(ip, port) {
                        Some(handle) => {
                            let hello = format!("HELLO {name}\n");
                            net::tcp_write(handle, hello.as_bytes());
                            kprintln!("CHAT: window open as '{name}' (relay {}:{port})", net::fmt_ip(&ip));
                            (ChatMode::Relay { handle, rx: Vec::new() }, vec![name.clone()])
                        }
                        None => {
                            kprintln!("CHAT: window open as '{name}' (relay connect failed, udp)");
                            (ChatMode::Udp, Vec::new())
                        }
                    },
                    None => {
                        kprintln!("CHAT: window open as '{name}' (udp broadcast :7777)");
                        (ChatMode::Udp, Vec::new())
                    }
                };
                self.add_window(
                    "chat",
                    40,
                    380,
                    440,
                    300,
                    App::Chat(ChatState {
                        name,
                        input: String::new(),
                        lines: Vec::new(),
                        users,
                        dm_target: None,
                        mode,
                    }),
                );
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
            "gif" => {
                let st = gifplayer::GifPlayerState::new();
                let title = st.title();
                self.add_window(&title, 200, 90, 280, 240, App::Gif(st));
                kprintln!("GIF: window open");
            }
            "files" => {
                self.add_window("files", 120, 60, 320, 378, App::Files(files::FilesState::new()));
                kprintln!("FILES: window open");
            }
            "lisp" => {
                self.add_window("lisp", 150, 70, 480, 320, App::Lisp(repl::LispState::new()));
                kprintln!("LISP: window open");
            }
            "snake" => {
                self.add_window("snake", 320, 90, 18 * 14, 24 + 18 * 14, App::Snake(snake::SnakeState::new()));
            }
            "breakout" => {
                self.add_window("breakout", 360, 70, 280, 24 + 320, App::Breakout(breakout::BreakoutState::new()));
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
            | App::Clock(_) | App::Chat(_) | App::Viewer(_) | App::Audio(_) | App::Files(_)
            | App::Gif(_) | App::Lisp(_) | App::Snake(_) | App::Video(_) | App::Wasm(_) | App::Breakout(_) => {}
        }
        if matches!(win.app, App::Snake(_)) {
            snake::render(&mut win);
        }
        if matches!(win.app, App::Video(_)) {
            video::render(&mut win);
        }
        if matches!(win.app, App::Wasm(_)) {
            wasmapp::render(&mut win);
        }
        if matches!(win.app, App::Breakout(_)) {
            breakout::render(&mut win);
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
        if matches!(win.app, App::Chat(_)) {
            render_chat(&mut win);
        }
        if matches!(win.app, App::Viewer(_)) {
            viewer::render(&mut win);
        }
        if matches!(win.app, App::Audio(_)) {
            render_audio(&mut win);
        }
        if matches!(win.app, App::Files(_)) {
            files::render(&mut win);
        }
        if matches!(win.app, App::Gif(_)) {
            gifplayer::render(&mut win);
        }
        if matches!(win.app, App::Lisp(_)) {
            repl::render(&mut win);
        }
        self.windows.push(win);
        self.dirty = true;
    }

    /// M29: open a FAT16 file in the app that handles its type (file
    /// manager dispatch). Raises the window if that file is already open.
    pub fn open_file(&mut self, name: &str) {
        if let Some(i) = self.windows.iter().position(|w| w.title == name) {
            self.raise(i);
            return;
        }
        if name.ends_with(".PNG") || name.ends_with(".JPG") || name.ends_with(".JPEG") {
            self.add_window(name, 220, 80, 560, 460, App::Viewer(viewer::ViewerState::with_file(name)));
            kprintln!("FILES: open {name} in Viewer");
        } else if name.ends_with(".WAV") {
            let st = AudioState {
                file: String::from(name),
                start_tick: 0,
                last_secs: u64::MAX,
                was_playing: false,
            };
            self.add_window(name, 360, 300, 300, 130, App::Audio(st));
            kprintln!("FILES: open {name} in Audio");
        } else if name.ends_with(".TXT") {
            self.add_window(name, 60, 60, 420, 300, App::Editor(EditorState::open(name)));
            kprintln!("FILES: open {name} in Editor");
        } else if name.ends_with(".GIF") {
            self.add_window(name, 200, 90, 280, 240, App::Gif(gifplayer::GifPlayerState::with_file(name)));
            kprintln!("FILES: open {name} in GIF player");
        } else if name.ends_with(".MJP") || name.ends_with(".AVI") {
            self.add_window(name, 200, 80, 360, 300, App::Video(video::VideoState::with_file(name)));
            kprintln!("FILES: open {name} in Video player");
        } else if name.ends_with(".WSM") {
            self.add_window(name, 180, 80, 460, 240, App::Wasm(wasmapp::WasmState::with_file(name)));
            kprintln!("FILES: open {name} in WASM runtime");
        } else {
            kprintln!("FILES: no app registered for {name}");
        }
        self.dirty = true;
    }

    // --- raw evdev event intake ---------------------------------------

    pub fn handle(&mut self, ev_type: u16, code: u16, value: u32) {
        match ev_type {
            keymap::EV_KEY => match code {
                keymap::KEY_LEFTSHIFT | keymap::KEY_RIGHTSHIFT => self.shift = value != 0,
                keymap::KEY_LEFTCTRL | keymap::KEY_RIGHTCTRL => self.ctrl = value != 0,
                keymap::KEY_LEFTALT => self.alt = value != 0,
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
            keymap::EV_REL if code == keymap::REL_WHEEL => {
                // Mouse-wheel scroll, routed to the focused browser window.
                if let Some(win) = self.windows.last_mut() {
                    if matches!(win.app, App::Browser(_)) && browser::wheel(win, value as i32) {
                        self.dirty = true;
                    }
                }
            }
            keymap::EV_SYN => self.commit(),
            _ => {}
        }
    }

    fn on_key(&mut self, code: u16) {
        // M35: Alt+Tab task switch; Ctrl+C/V/A clipboard. These take priority
        // over per-app key handling.
        if self.alt && code == keymap::KEY_TAB {
            self.cycle_windows();
            return;
        }
        if self.ctrl {
            match code {
                keymap::KEY_C | keymap::KEY_A => {
                    self.clipboard_copy();
                    return;
                }
                keymap::KEY_V => {
                    self.clipboard_paste();
                    return;
                }
                _ => {}
            }
        }
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
            // M31: GIF player — space/arrows handled in-window, Escape closes.
            if matches!(win.app, App::Gif(_)) {
                match gifplayer::key(win, code) {
                    gifplayer::Action::Redraw => {
                        self.dirty = true;
                        return;
                    }
                    gifplayer::Action::Close => {
                        if let Some(w) = self.windows.pop() {
                            kprintln!("WM: closed '{}'", w.title);
                        }
                        self.dirty = true;
                        return;
                    }
                    gifplayer::Action::None => {}
                }
            }
            // M32: Lisp REPL — Up/Down recall history, Page keys scroll output.
            if matches!(win.app, App::Lisp(_)) && repl::key(win, code) {
                repl::render(win);
                self.dirty = true;
                return;
            }
            // M35 Snake: arrow / WASD direction keys.
            if matches!(win.app, App::Snake(_)) && snake::key(win, code) {
                self.dirty = true;
                return;
            }
            if matches!(win.app, App::Breakout(_)) && breakout::key(win, code) {
                self.dirty = true;
                return;
            }
            // M35 Video: space play/pause, left/right seek.
            if matches!(win.app, App::Video(_)) && video::key(win, code) {
                self.dirty = true;
                return;
            }
            // M35 shell: Up/Down history, Tab completion (non-character keys).
            if matches!(win.app, App::Shell { .. }) && shell_raw_key(win, code) {
                self.dirty = true;
                return;
            }
            // M29: file-manager up/down/Enter (Enter dispatches via open_file).
            if matches!(win.app, App::Files(_)) {
                match files::key(win, code) {
                    files::Action::Redraw => {
                        self.dirty = true;
                        return;
                    }
                    files::Action::Open(name) => {
                        self.open_file(&name);
                        return;
                    }
                    files::Action::None => {}
                }
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
                App::Chat(_) => {
                    chat_key(win, ch);
                    self.dirty = true;
                }
                App::Lisp(_) => {
                    repl::char_input(win, ch);
                    repl::render(win);
                    self.dirty = true;
                }
                App::Browser(_) => {
                    // Address bar / focused form field text entry.
                    if browser::char_input(win, ch) {
                        self.dirty = true;
                    }
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
    /// Kill an app by window id (index) or name. Removing the window drops its
    /// App state — and thus its heap allocations (page buffers, etc.) — without
    /// touching any other app, so the others keep running.
    fn kill_app(&mut self, target: &str) -> bool {
        let by_id = target.parse::<usize>().ok();
        let pos = self.windows.iter().position(|w| {
            by_id == Some(usize::MAX) // never
                || w.title.eq_ignore_ascii_case(target)
                || w.title.to_ascii_lowercase().contains(&target.to_ascii_lowercase())
        });
        let pos = match (by_id, pos) {
            (Some(i), _) if i < self.windows.len() => Some(i),
            (_, p) => p,
        };
        let Some(i) = pos else { return false };
        let w = self.windows.remove(i);
        let freed = match &w.app {
            App::Browser(_) => "browser page buffers",
            App::Viewer(_) => "image buffer",
            _ => "app state",
        };
        kprintln!("WM: killed '{}' (reclaimed {freed})", w.title);
        self.dirty = true;
        true
    }

    /// Alt+Tab: bring the next window to the top (cycle the z-order).
    fn cycle_windows(&mut self) {
        if self.windows.len() < 2 {
            return;
        }
        let top = self.windows.pop().unwrap();
        self.windows.insert(0, top);
        if let Some(w) = self.windows.last() {
            kprintln!("WM: Alt+Tab -> '{}'", w.title);
        }
        self.dirty = true;
    }

    /// Ctrl+C / Ctrl+A: copy the focused app's text to the clipboard.
    fn clipboard_copy(&mut self) {
        let Some(win) = self.windows.last() else { return };
        match &win.app {
            App::Browser(_) => {
                let n = browser::copy_text(win);
                kprintln!("BROWSER: Ctrl+C copied {n} bytes of page text");
            }
            App::Shell { lines, .. } => {
                clipboard::set(lines.iter().cloned().collect());
            }
            App::Lisp(_) => {
                clipboard::set(repl::output_text(win));
            }
            App::Files(_) => {
                if let Some(name) = files::selected_name(win) {
                    clipboard::set(name);
                }
            }
            App::Editor(_) => {}
            _ => {}
        }
        self.dirty = true;
    }

    /// Ctrl+V: paste the clipboard into the focused app.
    fn clipboard_paste(&mut self) {
        let text = clipboard::get();
        if text.is_empty() {
            return;
        }
        let Some(win) = self.windows.last_mut() else { return };
        match &mut win.app {
            App::Shell { input, .. } => {
                input.push_str(text.trim());
                render_shell(win);
            }
            App::Browser(_) => {
                browser::paste(win);
            }
            App::Lisp(_) => {
                for c in text.chars().filter(|c| *c != '\n') {
                    repl::char_input(win, c);
                }
                repl::render(win);
            }
            _ => {}
        }
        kprintln!("CLIPBOARD: pasted {} bytes", text.len());
        self.dirty = true;
    }

    fn shell_execute(&mut self, cmd: &str) {
        kprintln!("SHELL: $ {cmd}");
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let first = cmd.split_whitespace().next().unwrap_or("");
        // `run <app>` or a bare launcher name opens a GUI app.
        let target = if first == "run" {
            cmd.split_whitespace().nth(1).map(String::from)
        } else if launcher_name(first).is_some() {
            Some(String::from(first))
        } else {
            None
        };
        if let Some(app) = target {
            self.launch(&app);
            self.shell_append(&format!("launched {app}\n"));
            return;
        }
        // ps / kill operate on the live window registry (each window is an app
        // with its own state + heap; dropping it reclaims memory, no reboot).
        if first == "ps" {
            let mut out = String::from("ID  APP\n");
            for (i, w) in self.windows.iter().enumerate() {
                out.push_str(&format!("{i:<3} {} (running)\n", w.title));
            }
            self.shell_append(&out);
            return;
        }
        if first == "kill" {
            let arg = cmd.split_whitespace().nth(1).unwrap_or("");
            if self.kill_app(arg) {
                self.shell_append(&format!("killed {arg}\n"));
            } else {
                self.shell_append(&format!("kill: {arg}: no such app\n"));
            }
            return;
        }
        // The real shell engine (ls/cat/cp/mv/rm/echo/pipes/...) is authoritative
        // for its built-ins; only an *unknown* command falls back to a user
        // binary on disk (e.g. `spin 5`, `hello`).
        let outcome = shell::run(cmd);
        if outcome.out.ends_with(": command not found\n") {
            let bin = format!("{}.BIN", first.to_ascii_uppercase());
            if let Some(image) = fs::read_file(&bin) {
                let args = cmd.split_once(' ').map(|x| x.1.trim()).unwrap_or("");
                match scheduler::spawn(&image, first, args) {
                    Some(pid) => self.shell_append(&format!("[{pid}] {first} started\n")),
                    None => self.shell_append("spawn failed (out of memory?)\n"),
                }
                return;
            }
        }
        if outcome.clear {
            if let Some(win) = self.windows.iter_mut().find(|w| matches!(w.app, App::Shell { .. })) {
                if let App::Shell { lines, .. } = &mut win.app {
                    lines.clear();
                }
            }
            self.dirty = true;
            return;
        }
        if !outcome.out.is_empty() {
            for line in outcome.out.lines() {
                kprintln!("SHELL_OUT: {line}");
            }
            self.shell_append(&outcome.out);
        }
        if let Some(app) = outcome.launch {
            self.launch(&app);
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
            } else if self.icon_drag.is_some() {
                // The floating icon follows the cursor; just need a recompose
                // (dirty already set above).
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
                        if ry < browser::TOPBAR as isize && rx < 18 {
                            browser::back(win); // the `<` back button
                        } else if browser::chrome_click(win, rx, ry) {
                            // address bar focused for editing
                        } else if browser::focus_field(win, rx, ry) {
                            // an on-page input field took focus
                        } else if let Some(href) = browser::link_at(win, rx, ry) {
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
                    App::Chat(_) => {
                        chat_click(win, rx, ry);
                        self.dirty = true;
                    }
                    App::Files(_) => match files::click(win, rx, ry) {
                        files::Action::Open(name) => self.open_file(&name),
                        files::Action::Redraw => self.dirty = true,
                        files::Action::None => {}
                    },
                    App::Gif(_) => {
                        gifplayer::click(win);
                        self.dirty = true;
                    }
                    _ => {}
                }
            }
            None => {
                // Bare desktop: arm a tap/hold on the icon under the cursor.
                // A quick release launches it (on_left_up); holding ~200ms
                // promotes to a drag (icon_tick).
                if let Some(slot) = self.icon_slot_at(self.mx, self.my) {
                    self.icon_press = Some((slot, timer::ticks()));
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

    /// Which icon slot (index into `icon_order`) a point lands on, if any —
    /// the exact-hit test used to start a tap/hold.
    fn icon_slot_at(&self, px: isize, py: isize) -> Option<usize> {
        let n = self.icon_order.len();
        let col0 = n.div_ceil(2);
        for i in 0..n {
            let (x, y) = icon_slot_xy(i, col0);
            if px >= x && px < x + ICON_W && py >= y && py < y + ICON_W {
                return Some(i);
            }
        }
        None
    }

    /// The nearest slot to drop a dragged icon onto (clamped to the grid).
    fn icon_drop_target(&self, px: isize, py: isize) -> usize {
        let n = self.icon_order.len();
        let col0 = n.div_ceil(2);
        // Column split at the midpoint of the gap between the two columns.
        let col0_hit = px < (ICON_COL0_X + ICON_COL1_X + ICON_W) / 2;
        let row = ((py - ICON_TOP).max(0) / ICON_SLOT) as usize;
        let idx = if col0_hit { row.min(col0.saturating_sub(1)) } else { col0 + row };
        idx.min(n.saturating_sub(1))
    }

    /// Persist the current icon order, one app name per line.
    fn save_icon_order(&self) {
        if !crate::fs::mounted() {
            return;
        }
        let mut s = String::new();
        for app in &self.icon_order {
            s.push_str(app);
            s.push('\n');
        }
        let _ = crate::fs::write_file(ICONS_FILE, s.as_bytes());
    }

    fn on_left_up(&mut self) {
        // Complete an icon drag: reorder, persist, DRAG_OK.
        if let Some(from) = self.icon_drag.take() {
            self.icon_press = None;
            if from < self.icon_order.len() {
                let target = self.icon_drop_target(self.mx, self.my);
                let app = self.icon_order.remove(from);
                let target = target.min(self.icon_order.len());
                self.icon_order.insert(target, app);
                self.save_icon_order();
                kprintln!("WM: icon '{app}' dropped at slot {target}");
                kprintln!("DRAG_OK");
            }
            self.dirty = true;
            return;
        }
        // A quick tap on an icon (no hold-to-drag) launches it.
        if let Some((slot, _)) = self.icon_press.take() {
            if let Some(app) = self.icon_order.get(slot).copied() {
                kprintln!("WM: icon -> '{app}'");
                self.launch(app);
            }
            return;
        }
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

    /// Promote a held icon press to a drag once the button has been down on it
    /// for ~200 ms (distinguishing a hold-to-drag from a tap-to-launch). Called
    /// each desktop-loop iteration; works even with no motion events since the
    /// 50 Hz tick keeps waking the loop.
    pub fn icon_tick(&mut self) {
        if self.icon_drag.is_some() {
            return;
        }
        if let Some((slot, t0)) = self.icon_press {
            if self.buttons & 1 != 0 && timer::ticks().saturating_sub(t0) >= ICON_HOLD_TICKS {
                self.icon_drag = Some(slot);
                self.dirty = true;
                let app = self.icon_order.get(slot).copied().unwrap_or("?");
                kprintln!("WM: icon drag start '{app}'");
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
            if matches!(win.app, App::Gif(_)) && gifplayer::tick(win, now) {
                self.dirty = true;
            }
            if matches!(win.app, App::Snake(_)) && snake::tick(win, now) {
                self.dirty = true;
            }
            if matches!(win.app, App::Breakout(_)) && breakout::tick(win, now) {
                self.dirty = true;
            }
            if matches!(win.app, App::Video(_)) && video::tick(win, now) {
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
        // Subtle grid texture on the desktop background.
        let mut gx = 0;
        while gx < w {
            back.fill_rect(gx, 0, 1, h, DESKTOP_GRID);
            gx += 48;
        }
        let mut gy = 0;
        while gy < h {
            back.fill_rect(0, gy, w, 1, DESKTOP_GRID);
            gy += 48;
        }

        // Desktop icons: two-column grid in the user-defined order. Each slot
        // is 68px (48 icon + 12 label + 8 gap). The icon being dragged is drawn
        // floating at the cursor (below), not in its home slot.
        let iw = ICON_W as usize;
        let col0_count = self.icon_order.len().div_ceil(2);
        for (i, app) in self.icon_order.iter().enumerate() {
            if self.icon_drag == Some(i) {
                continue;
            }
            let (cx, cy) = icon_slot_xy(i, col0_count);
            let (cx, cy) = (cx as usize, cy as usize);
            // Rounded-rect tile in the app's colour, with a subtle inner sheen.
            back.fill_round_rect(cx, cy, iw, iw, 8, icon_color(app));
            back.fill_round_rect(cx + 2, cy + 2, iw - 4, iw / 3, 6, blend(icon_color(app), 0xffffffff, 36));
            // Two-letter glyph, centred, in bold Barlow.
            ui_centered(&back, cx + iw / 2, cy + 9, icon_abbrev(app), font::ui_icon(), 0xfff4f4f4);
            // Full app name below.
            ui_centered(&back, cx + iw / 2, cy + iw + 1, icon_label(app), font::ui_small(), 0xffc8ccd4);
        }

        let top = self.windows.len().saturating_sub(1);
        for (idx, win) in self.windows.iter().enumerate() {
            let focused = idx == top;
            // Soft drop shadow (offset, semi-transparent) behind the window.
            {
                let sx = (win.x + 3).max(0) as usize;
                let sy = (win.y + 3).max(0) as usize;
                let sw = (win.x + win.frame_w() + 3).min(w as isize) - sx as isize;
                let sh = (win.y + win.frame_h() + 3).min(h as isize) - sy as isize;
                if sw > 0 && sh > 0 {
                    back.blend_rect(sx, sy, sw as usize, sh as usize, 0xff00_0000, 90);
                }
            }
            // Frame (thin border) as one filled rect behind everything; the
            // focused window's border is the accent colour.
            if win.x + win.frame_w() > 0 && win.y + win.frame_h() > 0 {
                let fx = win.x.max(0) as usize;
                let fy = win.y.max(0) as usize;
                let fw = (win.x + win.frame_w()).min(w as isize) - fx as isize;
                let fh = (win.y + win.frame_h()).min(h as isize) - fy as isize;
                if fw > 0 && fh > 0 {
                    back.fill_rect(fx, fy, fw as usize, fh as usize, if focused { FRAME_FOCUSED } else { FRAME_COLOR });
                }
            }
            // Title bar: a touch lighter than the body, Barlow title, a circular
            // close button, and an accent underline on the focused window.
            let tx = win.x + BORDER;
            let ty = win.y + BORDER;
            if tx >= 0 && ty >= 0 {
                let (txu, tyu) = (tx as usize, ty as usize);
                back.fill_rect(txu, tyu, win.cw, TITLE_H as usize, if focused { 0xff222222 } else { 0xff181818 });
                if focused {
                    back.fill_rect(txu, (ty + TITLE_H - 1) as usize, win.cw, 1, ACCENT);
                }
                let tcol = if focused { 0xffe8e8e8 } else { 0xff666666 };
                back.draw_bm_string(txu + 10, tyu + 2, &win.title, font::ui(), tcol);
                // Close button: a 5px-radius circle near the right edge.
                let ccx = (tx + win.cw as isize - 12) as isize;
                let ccy = ty + TITLE_H / 2;
                let close_col = if focused { 0xffe05555 } else { 0xff555555 };
                back.fill_circle(ccx, ccy, 5, close_col);
            }
            // Content.
            back.blit(win.x + BORDER, win.y + BORDER + TITLE_H, &win.canvas, win.cw, win.ch);
            // Round the outer corners of the whole window (mask to desktop bg).
            if win.x >= 0 && win.y >= 0 {
                let fw = win.frame_w() as usize;
                let fh = win.frame_h() as usize;
                back.round_corners(win.x as usize, win.y as usize, fw, fh, 4, DESKTOP_BG);
            }
        }

        // Taskbar: a slim dark strip with pill launchers, a wordmark, a clock.
        let ty = h - TASKBAR_H;
        back.fill_rect(0, ty, w, TASKBAR_H, 0xff111111);
        back.fill_rect(0, ty, w, 1, 0xff262626); // hairline top edge
        ui_text(&back, 12, ty + 7, "VEIL", font::ui_bold(), ACCENT);
        let mut bx = 70usize;
        for (app, label) in launchers() {
            // Keep the 70/+78 layout (hit-test compatible); draw a pill inside it.
            let open = self.windows.iter().any(|w| w.title == app);
            let (pill, txt) = if open {
                (blend(0xff111111, ACCENT, 70), ACCENT)
            } else {
                (0xff1e1e1e, 0xff8a8a8a)
            };
            back.fill_round_rect(bx, ty + 5, 72, TASKBAR_H - 10, 7, pill);
            ui_centered(&back, bx + 36, ty + 7, label, font::ui_small(), txt);
            bx += 78;
        }
        // Clock on the far right.
        let clk = clock_string();
        let cw_px = font::text_width(font::ui_small(), &clk);
        back.draw_bm_string(w - cw_px - 14, ty + 7, &clk, font::ui_small(), 0xffb0b0b0);

        // The dragged icon floats semi-transparent under the cursor.
        if let Some(slot) = self.icon_drag {
            if let Some(app) = self.icon_order.get(slot) {
                let iw = ICON_W as usize;
                let fx = (self.mx - ICON_W / 2).max(0) as usize;
                let fy = (self.my - ICON_W / 2).max(0) as usize;
                back.blend_rect(fx, fy, iw, iw, icon_color(*app), 150);
                ui_centered(&back, fx + iw / 2, fy + 9, icon_abbrev(app), font::ui_icon(), 0xfff4f4f4);
            }
        }

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
    let App::Shell { input, lines, history, hist } = &mut win.app else {
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
            if !cmd.trim().is_empty() && history.last() != Some(&cmd) {
                history.push(cmd.clone());
            }
            *hist = history.len();
            command = Some(cmd);
        }
        c => input.push(c),
    }
    render_shell(win);
    command
}

/// Up/Down recall command history; Tab completes a filename. Returns true if
/// the key was consumed.
fn shell_raw_key(win: &mut Window, code: u16) -> bool {
    const KEY_TAB: u16 = 15;
    const KEY_UP: u16 = 103;
    const KEY_DOWN: u16 = 108;
    let App::Shell { input, history, hist, .. } = &mut win.app else {
        return false;
    };
    match code {
        KEY_UP => {
            if *hist > 0 {
                *hist -= 1;
                *input = history[*hist].clone();
            }
        }
        KEY_DOWN => {
            if *hist + 1 < history.len() {
                *hist += 1;
                *input = history[*hist].clone();
            } else {
                *hist = history.len();
                input.clear();
            }
        }
        KEY_TAB => {
            // Complete the last whitespace-separated token against disk names.
            let (head, frag) = match input.rsplit_once(char::is_whitespace) {
                Some((h, f)) => (alloc::format!("{h} "), f.to_string()),
                None => (String::new(), input.clone()),
            };
            let matches = shell::complete(&frag);
            if matches.len() == 1 {
                *input = format!("{head}{}", matches[0]);
            }
        }
        _ => return false,
    }
    render_shell(win);
    true
}

fn render_shell(win: &mut Window) {
    let mono = font::pick(font::Family::Mono, 400, false, 16);
    let lh = mono.map(|f| f.height as usize + 2).unwrap_or(16);
    let (input, visible) = {
        let App::Shell { input, lines, .. } = &win.app else { return };
        let rows = win.ch.saturating_sub(lh + 8) / lh;
        let skip = lines.len().saturating_sub(rows);
        (input.clone(), lines[skip..].to_vec())
    };
    let fb = win.canvas_fb();
    fb.clear(0xff14_1414);
    let chev = font::text_width(mono.unwrap_or(font::ui_small()), "> ");
    let is_err = |s: &str| s.contains("not found") || s.contains("no such") || s.contains("failed") || s.contains(": error");
    let draw = |fb: &Framebuffer, x: usize, y: usize, s: &str, c: u32| match mono {
        Some(m) => fb.draw_bm_string(x, y, s, m, c),
        None => fb.draw_string(x, y, s, c, None),
    };
    let mut y = 6;
    for line in &visible {
        let text = line.trim_end_matches('\n');
        if let Some(cmd) = text.strip_prefix("> ") {
            draw(&fb, 6, y, ">", ACCENT); // chevron prompt
            draw(&fb, 6 + chev, y, cmd, 0xffff_ffff);
        } else {
            draw(&fb, 6, y, text, if is_err(text) { 0xffe0_5555 } else { 0xffcc_cccc });
        }
        y += lh;
    }
    let py = win.ch.saturating_sub(lh + 2);
    draw(&fb, 6, py, ">", ACCENT);
    draw(&fb, 6 + chev, py, &format!("{input}_"), 0xffff_ffff);
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
const CHAT_DM: u32 = 0xffb0_5838; // terracotta — direct messages
const CHAT_PROMPT: u32 = 0xff60_3080;
// Right-side online-user panel (M26): fixed width, green dot per user.
const PANEL_W: usize = 80;
const PANEL_BG: u32 = 0xffe8_e6e0;
const PANEL_TEXT: u32 = 0xff30_3840;
const PANEL_SEL: u32 = 0xffc0_7850; // highlighted DM target
const PANEL_DOT: u32 = 0xff40_b060; // online indicator
static CHAT_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static DM_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn chat_ok() {
    use core::sync::atomic::Ordering;
    if !CHAT_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("CHAT_OK");
    }
}

fn dm_ok() {
    use core::sync::atomic::Ordering;
    if !DM_DONE.swap(true, Ordering::Relaxed) {
        kprintln!("DM_OK");
    }
}

/// The chat sender label (M25). Priority: USER.TXT on the FAT16 disk (the
/// session manager / first-boot setup writes the visitor's name there),
/// then the legacy local-IP A/B convention (the diskless M20 two-instance
/// proof has no USER.TXT), then a random 6-hex-char id derived from the
/// boot-time hardware timer.
fn chat_username() -> String {
    if let Some(data) = fs::read_file("USER.TXT") {
        if let Ok(s) = core::str::from_utf8(&data) {
            let name: String = s.trim().chars().take(20).collect();
            if !name.is_empty() {
                return name;
            }
        }
    }
    match net::local_ip() {
        Some([_, _, _, 1]) => return String::from("A"),
        Some([_, _, _, 2]) => return String::from("B"),
        _ => {}
    }
    let now: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) now, options(nomem, nostack)) };
    format!("{:06x}", now & 0xff_ffff)
}

fn render_chat(win: &mut Window) {
    let App::Chat(st) = &win.app else { return };
    let (cw, ch) = (win.cw, win.ch);
    // Relay mode reserves a right-hand user panel; UDP mode uses full width.
    let relay = matches!(st.mode, ChatMode::Relay { .. });
    let log_w = if relay { cw - PANEL_W } else { cw };
    let rows = (ch - 28) / 16;
    let skip = st.lines.len().saturating_sub(rows);
    let prompt = match &st.dm_target {
        Some(t) => format!("@{t}> {}_", st.input),
        None => format!("{}> {}_", st.name, st.input),
    };
    let visible: Vec<(String, u32)> =
        st.lines[skip..].iter().map(|l| (l.text.clone(), l.color)).collect();
    let users = st.users.clone();
    let target = st.dm_target.clone();

    let fb = win.canvas_fb();
    fb.clear(CHAT_BG);
    for (i, (text, color)) in visible.iter().enumerate() {
        // Clip a long line to the log width (8px per glyph).
        let max_cols = log_w.saturating_sub(8) / 8;
        let shown: String = text.chars().take(max_cols).collect();
        fb.draw_string(6, 4 + 16 * i, &shown, *color, None);
    }
    fb.draw_string(6, ch - 20, &prompt, CHAT_PROMPT, None);

    if relay {
        let px = log_w;
        fb.fill_rect(px, 0, PANEL_W, ch, PANEL_BG);
        fb.draw_string(px + 6, 4, "online", PANEL_TEXT, None);
        for (i, u) in users.iter().enumerate() {
            let y = 24 + i * 16;
            if y + 14 > ch {
                break;
            }
            if target.as_deref() == Some(u.as_str()) {
                fb.fill_rect(px + 2, y - 1, PANEL_W - 4, 15, PANEL_SEL);
            }
            fb.fill_rect(px + 6, y + 4, 6, 6, PANEL_DOT);
            let label: String = u.chars().take((PANEL_W - 18) / 8).collect();
            fb.draw_string(px + 16, y, &label, PANEL_TEXT, None);
        }
    }
}

/// Content-area click inside a chat window (relay mode): a click on the
/// right user panel selects/deselects that user as the DM target.
fn chat_click(win: &mut Window, rx: isize, ry: isize) {
    let App::Chat(st) = &mut win.app else { return };
    if !matches!(st.mode, ChatMode::Relay { .. }) {
        return;
    }
    let panel_x = (win.cw - PANEL_W) as isize;
    if rx < panel_x {
        return; // log area, not the panel
    }
    let i = (ry - 24) / 16;
    if i < 0 {
        return;
    }
    let Some(u) = st.users.get(i as usize).cloned() else { return };
    // Toggle: click the current target (or your own name) to go back to the
    // public room.
    st.dm_target = if st.dm_target.as_deref() == Some(u.as_str()) || u == st.name {
        None
    } else {
        kprintln!("CHAT: dm target -> {u}");
        Some(u)
    };
    render_chat(win);
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

/// Push a log line, capping scrollback.
fn chat_push(st: &mut ChatState, text: String, color: u32) {
    st.lines.push(ChatLine { text, color });
    let excess = st.lines.len().saturating_sub(200);
    if excess > 0 {
        st.lines.drain(..excess);
    }
}

fn chat_key(win: &mut Window, ch: char) {
    {
        let App::Chat(st) = &mut win.app else { return };
        match ch {
            '\u{8}' => {
                st.input.pop();
            }
            '\n' => {
                let text = st.input.trim().to_string();
                st.input.clear();
                if !text.is_empty() {
                    chat_send_line(st, &text);
                }
            }
            c => {
                if st.input.len() < 120 {
                    st.input.push(c);
                }
            }
        }
    }
    render_chat(win);
}

/// Send the typed line, either as a relay MSG frame (public or DM to the
/// current target) or as an M20 UDP broadcast.
fn chat_send_line(st: &mut ChatState, text: &str) {
    match &mut st.mode {
        ChatMode::Relay { handle, .. } => {
            let to = st.dm_target.clone().unwrap_or_else(|| String::from("*"));
            let body = &text.as_bytes()[..text.len().min(400)];
            let header = format!("MSG {} {} {}\n", st.name, to, body.len());
            let mut frame = header.into_bytes();
            frame.extend_from_slice(body);
            net::tcp_write(*handle, &frame);
            kprintln!("CHAT: sent MSG to '{to}' ({} bytes)", body.len());
            chat_ok();
            // The relay echoes our own message back, so we render on receipt
            // (keeps DM/public colouring identical in both directions).
        }
        ChatMode::Udp => {
            let mut msg = format!("{}: {}\n", st.name, text);
            msg.truncate(128);
            if net::chat_send(msg.as_bytes()) {
                kprintln!("CHAT: sent {} bytes: {}", msg.len(), msg.trim_end());
                chat_ok();
            } else {
                kprintln!("CHAT: send failed (no netstack?)");
            }
            chat_push(st, msg.trim_end().to_string(), CHAT_MINE);
        }
    }
}

/// One relay protocol event parsed off the wire.
enum RelayEvent {
    Join(String),
    Part(String),
    Msg { from: String, to: String, body: String },
}

/// Drain complete frames out of the relay receive buffer. Lines are
/// newline-terminated; an `MSG <from> <to> <len>\n` header is followed by
/// exactly `<len>` raw payload bytes (which we leave intact until present).
fn parse_relay(rx: &mut Vec<u8>) -> Vec<RelayEvent> {
    let mut out = Vec::new();
    loop {
        let Some(nl) = rx.iter().position(|&b| b == b'\n') else { break };
        let line = String::from_utf8_lossy(&rx[..nl]).into_owned();
        let mut it = line.split(' ');
        match it.next() {
            Some("MSG") => {
                let from = it.next().unwrap_or("").to_string();
                let to = it.next().unwrap_or("").to_string();
                let len: usize = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                if rx.len() < nl + 1 + len {
                    break; // payload not fully arrived yet
                }
                let body = String::from_utf8_lossy(&rx[nl + 1..nl + 1 + len]).into_owned();
                rx.drain(..nl + 1 + len);
                out.push(RelayEvent::Msg { from, to, body });
            }
            Some("JOIN") => {
                if let Some(u) = it.next() {
                    out.push(RelayEvent::Join(u.to_string()));
                }
                rx.drain(..nl + 1);
            }
            Some("PART") => {
                if let Some(u) = it.next() {
                    out.push(RelayEvent::Part(u.to_string()));
                }
                rx.drain(..nl + 1);
            }
            _ => {
                rx.drain(..nl + 1); // unknown line: skip
            }
        }
    }
    out
}

impl Wm {
    /// Append a received chat datagram (M20 UDP mode) to the chat log.
    pub fn chat_append(&mut self, msg: &str) {
        let Some(win) = self.windows.iter_mut().find(|w| matches!(w.app, App::Chat(_))) else {
            return;
        };
        {
            let App::Chat(st) = &mut win.app else { unreachable!() };
            for piece in msg.split_inclusive('\n') {
                if !piece.trim().is_empty() {
                    chat_push(st, piece.trim_end_matches('\n').to_string(), CHAT_TEXT);
                }
            }
        }
        kprintln!("CHAT: rx {:?}", msg.trim_end());
        chat_ok();
        render_chat(win);
        self.dirty = true;
    }

    /// M26: pump the relay TCP connection — read available bytes, parse
    /// HELLO/JOIN/PART/MSG frames, and update the log + online roster.
    /// Called every desktop-loop iteration.
    pub fn chat_poll(&mut self) {
        let Some(win) = self.windows.iter_mut().find(|w| matches!(w.app, App::Chat(_))) else {
            return;
        };
        let App::Chat(st) = &mut win.app else { return };
        let ChatMode::Relay { handle, rx } = &mut st.mode else { return };
        let handle = *handle;
        // Drain the socket into the per-window buffer.
        let mut tmp = [0u8; 1024];
        let mut got = false;
        loop {
            match net::tcp_read(handle, &mut tmp) {
                net::TcpRead::Data(n) => {
                    rx.extend_from_slice(&tmp[..n]);
                    got = true;
                }
                _ => break,
            }
        }
        if !got {
            return;
        }
        let events = parse_relay(rx);
        if events.is_empty() {
            return;
        }
        let name = st.name.clone();
        for ev in events {
            match ev {
                RelayEvent::Join(u) => {
                    if !st.users.contains(&u) {
                        st.users.push(u.clone());
                        st.users.sort();
                        kprintln!("CHAT: join {u} (users {})", st.users.len());
                    }
                }
                RelayEvent::Part(u) => {
                    st.users.retain(|x| *x != u);
                    kprintln!("CHAT: part {u} (users {})", st.users.len());
                }
                RelayEvent::Msg { from, to, body } => {
                    let (text, color) = if to == "*" {
                        let c = if from == name { CHAT_MINE } else { CHAT_TEXT };
                        (format!("{from}: {body}"), c)
                    } else {
                        let t = if from == name {
                            format!("{from} -> {to}: {body}")
                        } else {
                            format!("{from} -> you: {body}")
                        };
                        dm_ok();
                        (t, CHAT_DM)
                    };
                    kprintln!("CHAT: rx {text:?}");
                    chat_push(st, text, color);
                    chat_ok();
                }
            }
        }
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
