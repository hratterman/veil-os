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
    breakout, browser, calc, clipboard, clock, files, font, freetype::FontId, fs, gifplayer, keymap,
    kprintln, net, netdev, repl, scheduler, settings, shell, snake, snd, timer, video, viewer, wasmapp,
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
const TBTN_W: isize = 20; // width of each title-bar button (close/max/min)

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
const LAUNCHERS: [(&str, &str); 15] = [
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
    ("calc", "Calc"),
    ("settings", "Settings"),
];
// Per-app icon colours (modern, muted): browser blue, shell green, files amber,
// lisp purple, games teal/orange, etc. Indexed by LAUNCHERS order.
const ICON_COLORS: [u32; 15] = [
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
    0xff5a8a8a, // calc - steel
    0xff80808c, // settings - gray
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
        "calc" => "Ca",
        "settings" => "St",
        _ => "Ap",
    }
}

/// Draw an anti-aliased FreeType UI string; returns its pixel width.
fn ui_text(fb: &Framebuffer, x: usize, y: usize, s: &str, font: FontId, size: u16, color: u32) -> usize {
    fb.draw_text(x, y, s, font, size, color)
}

/// Draw a FreeType string centred on `cx`.
fn ui_centered(fb: &Framebuffer, cx: usize, y: usize, s: &str, font: FontId, size: u16, color: u32) {
    let (w, _) = fb.measure_text(s, font, size);
    fb.draw_text(cx.saturating_sub(w / 2), y, s, font, size, color);
}

/// Truncate `s` so it fits in `max_w` pixels at (font, size) (for narrow pills).
fn fit_label(fb: &Framebuffer, s: &str, max_w: usize, font: FontId, size: u16) -> String {
    if fb.measure_text(s, font, size).0 <= max_w {
        return String::from(s);
    }
    let mut out = String::new();
    for ch in s.chars() {
        let mut trial = out.clone();
        trial.push(ch);
        if fb.measure_text(&trial, font, size).0 > max_w {
            break;
        }
        out = trial;
    }
    out
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

const HAND: [&[u8]; 13] = [
    b"..XX....",
    b"..XOX...",
    b"..XOX...",
    b"..XOX...",
    b"..XOXXX.",
    b"XX.XOXOX",
    b"XOXXOXOX",
    b"XOXOOXOX",
    b"XXOOOOOX",
    b".XOOOOX.",
    b".XOOOOX.",
    b"..XOOOX.",
    b"..XXXX..",
];

// --- Wallpaper (M36): decoded once, scaled to fill the desktop ---------------
static mut WALLPAPER: Option<crate::png::Image> = None;
static mut WALLPAPER_ON: bool = false;

/// Toggle the wallpaper on/off, decoding WALLPAPER.PNG/JPG on first enable.
fn set_wallpaper_next() {
    unsafe {
        let on = &mut *core::ptr::addr_of_mut!(WALLPAPER_ON);
        *on = !*on;
        if *on && (*core::ptr::addr_of!(WALLPAPER)).is_none() && crate::fs::mounted() {
            for name in ["WALLPAPER.PNG", "WALLPAPER.JPG", "SUNSET.PNG", "PLASMA.PNG"] {
                if let Some(data) = crate::fs::read_file(name) {
                    if let Some(img) = crate::png::decode_any(&data) {
                        kprintln!("WALLPAPER: loaded {name} {}x{}", img.w, img.h);
                        *core::ptr::addr_of_mut!(WALLPAPER) = Some(img);
                        break;
                    }
                }
            }
        }
    }
}

/// Public wrapper so the settings app can toggle the wallpaper.
pub fn toggle_wallpaper() {
    set_wallpaper_next();
}

static WINDOW_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Number of open windows (for the settings "apps running" stat).
pub fn window_count() -> usize {
    WINDOW_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

fn wallpaper() -> Option<&'static crate::png::Image> {
    unsafe {
        if *core::ptr::addr_of!(WALLPAPER_ON) {
            (*core::ptr::addr_of!(WALLPAPER)).as_ref()
        } else {
            None
        }
    }
}

/// Render the pointer at (mx, my) in the requested shape. White fill with a
/// black outline so it reads on any background.
fn draw_cursor(fb: &Framebuffer, mx: isize, my: isize, shape: CursorShape) {
    const W: u32 = 0xffff_ffff;
    const B: u32 = 0xff00_0000;
    let put = |x: isize, y: isize, c: u32| {
        if x >= 0 && y >= 0 {
            fb.put_pixel(x as usize, y as usize, c);
        }
    };
    let blit = |bm: &[&[u8]], ox: isize, oy: isize| {
        for (r, line) in bm.iter().enumerate() {
            for (c, &ch) in line.iter().enumerate() {
                let col = match ch {
                    b'X' => B,
                    b'O' => W,
                    _ => continue,
                };
                put(mx + ox + c as isize, my + oy + r as isize, col);
            }
        }
    };
    // A double-headed arrow shaft from -len..len along (dx,dy) with arrowheads.
    let arrows = |dx: isize, dy: isize| {
        for t in -7..=7 {
            // shaft (white core, black outline on both sides)
            put(mx + dx * t, my + dy * t, W);
            put(mx + dx * t - dy, my + dy * t - dx, B);
            put(mx + dx * t + dy, my + dy * t + dx, B);
        }
        for (sx, sy) in [(dx, dy), (-dx, -dy)] {
            for i in 0..4 {
                let bx = mx + sx * (7 - i);
                let by = my + sy * (7 - i);
                // arrowhead wings perpendicular-ish
                put(bx + sy * i, by + sx * i, W);
                put(bx - sy * i, by - sx * i, W);
                put(bx + sy * (i + 1), by + sx * (i + 1), B);
                put(bx - sy * (i + 1), by - sx * (i + 1), B);
            }
        }
    };
    match shape {
        CursorShape::Arrow => blit(&CURSOR, 0, 0),
        CursorShape::Hand => blit(&HAND, -3, 0),
        CursorShape::IBeam => {
            for dy in -7..=7 {
                put(mx, my + dy, W);
                put(mx - 1, my + dy, B);
                put(mx + 1, my + dy, B);
            }
            for dx in -2..=2 {
                put(mx + dx, my - 7, W);
                put(mx + dx, my + 7, W);
                put(mx + dx, my - 8, B);
                put(mx + dx, my + 8, B);
            }
        }
        CursorShape::ResizeH => arrows(1, 0),
        CursorShape::ResizeV => arrows(0, 1),
        CursorShape::ResizeNWSE => arrows(1, 1),
        CursorShape::ResizeNESW => arrows(1, -1),
    }
}

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
    Calc(calc::CalcState),
    Settings(settings::SettingsState),
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
    pub minimized: bool,
    /// Saved (x, y, cw, ch) before maximize/snap, for restore.
    pub restore: Option<(isize, isize, usize, usize)>,
}

pub const MIN_CW: usize = 160;
pub const MIN_CH: usize = 120;

/// Re-render an app's content into its (possibly resized) canvas.
pub fn render_window_content(win: &mut Window) {
    match win.app {
        App::Snake(_) => snake::render(win),
        App::Video(_) => video::render(win),
        App::Wasm(_) => wasmapp::render(win),
        App::Breakout(_) => breakout::render(win),
        App::Shell { .. } => render_shell(win),
        App::Editor(_) => render_editor(win),
        App::Clock(_) => clock::render(win, timer::ticks()),
        App::Chat(_) => render_chat(win),
        App::Viewer(_) => viewer::render(win),
        App::Audio(_) => render_audio(win),
        App::Files(_) => files::render(win),
        App::Gif(_) => gifplayer::render(win),
        App::Lisp(_) => repl::render(win),
        App::Browser(_) => browser::paint_view(win),
        App::Calc(_) => calc::render(win),
        App::Settings(_) => settings::render(win),
        App::Paint(_) => {
            let (c, b) = if let App::Paint(p) = &win.app { (p.color, p.brush) } else { (0, 1) };
            let cw = win.cw;
            let fb = win.canvas_fb();
            render_paint_toolbar(&fb, cw, c, b);
        }
        App::Echo { .. } | App::Static => {}
    }
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

/// Pointer shapes the WM renders depending on what is under the cursor.
#[derive(Clone, Copy, PartialEq)]
pub enum CursorShape {
    Arrow,
    IBeam,
    Hand,
    ResizeH,    // <->
    ResizeV,    // up/down
    ResizeNWSE, // \ corner
    ResizeNESW, // / corner
}

// Resize edge bitmask.
const E_L: u8 = 1;
const E_R: u8 = 2;
const E_T: u8 = 4;
const E_B: u8 = 8;
const RESIZE_ZONE: isize = 7; // px hit zone outside/inside the frame edge

fn edge_cursor(edge: u8) -> CursorShape {
    if edge == E_L | E_T || edge == E_R | E_B {
        CursorShape::ResizeNWSE
    } else if edge == E_R | E_T || edge == E_L | E_B {
        CursorShape::ResizeNESW
    } else if edge == E_L || edge == E_R {
        CursorShape::ResizeH
    } else {
        CursorShape::ResizeV
    }
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
    resize: Option<(usize, u8)>,         // (window index AT TOP, edge bitmask LRTB)
    last_title_click: (u64, isize, isize), // (tick, x, y) for double-click detect
    cursor: CursorShape,
    shift: bool,
    ctrl: bool, // Ctrl held (clipboard shortcuts)
    alt: bool,  // Alt held (Alt+Tab task switch)
    abs_max: (u32, u32),
    pub dirty: bool,
    icon_order: Vec<&'static str>,     // desktop icon display order
    icon_press: Option<(usize, u64)>,  // (order slot, press tick) — pending tap/hold
    icon_drag: Option<usize>,          // order slot currently being dragged
    toasts: Vec<(String, u64)>,        // (message, expiry tick) bottom-right popups
    shot_seq: u32,                     // screenshot file counter
    menu: Option<ContextMenu>,         // open right-click menu
    flash: u64,                        // screenshot flash effect expiry tick
    file_drag: Option<FileDrag>,       // dragging a file out of the file manager
}

/// A file being dragged out of the file manager.
struct FileDrag {
    name: String,
    start: (isize, isize),
    active: bool, // true once moved past the drag threshold
}

/// A right-click context menu: items + screen anchor + the target it acts on.
pub struct ContextMenu {
    items: Vec<String>,
    x: isize,
    y: isize,
    target: MenuTarget,
}

#[derive(Clone)]
enum MenuTarget {
    Desktop,
    Window(usize),
    Clipboard,
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
            resize: None,
            last_title_click: (0, 0, 0),
            cursor: CursorShape::Arrow,
            shift: false,
            ctrl: false,
            alt: false,
            abs_max,
            dirty: true,
            icon_order: load_icon_order(),
            icon_press: None,
            icon_drag: None,
            toasts: Vec::new(),
            shot_seq: 0,
            menu: None,
            flash: 0,
            file_drag: None,
        }
    }

    /// Show a transient toast in the bottom-right (auto-dismiss ~3s).
    pub fn notify(&mut self, msg: &str) {
        kprintln!("NOTIFY: {msg}");
        let expiry = timer::ticks() + 150; // ~3s at 50Hz
        self.toasts.push((String::from(msg), expiry));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        self.dirty = true;
    }

    /// Capture a screenshot (full screen or focused window) to SHOT_NN.PNG.
    pub fn screenshot(&mut self, focused_only: bool) {
        let (src, sw, sh, what): (&[u32], usize, usize, &str) =
            if focused_only && !self.windows.is_empty() {
                let win = self.windows.last().unwrap();
                (&win.canvas, win.cw, win.ch, "window")
            } else {
                (&self.back, self.screen.width, self.screen.height, "screen")
            };
        // Downsample so the largest side is <= 512 — keeps the PNG heap-safe.
        let factor = (sw.max(sh) / 512).max(1);
        let (w, h) = (sw / factor, sh / factor);
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                pixels.push(src[(y * factor) * sw + x * factor]);
            }
        }
        let data = crate::png::encode(&pixels, w, h);
        self.shot_seq += 1;
        let name = format!("SHOT{:02}.PNG", self.shot_seq);
        if crate::fs::mounted() && crate::fs::write_file(&name, &data).is_ok() {
            kprintln!("SCREENSHOT_OK: {name} ({}x{} {} bytes, {what})", w, h, data.len());
            self.notify(&format!("Screenshot saved: {name}"));
        } else {
            self.notify("Screenshot failed (no disk)");
        }
        self.flash = timer::ticks() + 6; // brief white flash
        self.dirty = true;
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
            "calc" => self.add_window("calc", 380, 120, 264, 420, App::Calc(calc::CalcState::new())),
            "settings" => {
                self.add_window("settings", 240, 110, 480, 360, App::Settings(settings::SettingsState::new()));
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
            minimized: false,
            restore: None,
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
            | App::Gif(_) | App::Lisp(_) | App::Snake(_) | App::Video(_) | App::Wasm(_)
            | App::Breakout(_) | App::Calc(_) | App::Settings(_) => {}
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
        if matches!(win.app, App::Calc(_)) {
            calc::render(&mut win);
        }
        if matches!(win.app, App::Settings(_)) {
            settings::render(&mut win);
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
        } else if [".TXT", ".RS", ".PY", ".JS", ".CSS", ".SH", ".MD", ".LOG", ".TOML", ".JSON"]
            .iter()
            .any(|e| name.ends_with(e))
        {
            self.add_window(name, 60, 60, 460, 360, App::Editor(EditorState::open(name)));
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
                keymap::KEY_SYSRQ if value == 1 => self.screenshot(self.alt),
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
        // Global window shortcuts.
        if (self.ctrl && code == keymap::KEY_W) || (self.alt && code == keymap::KEY_F4) {
            if let Some(w) = self.windows.pop() {
                kprintln!("WM: closed '{}' (shortcut)", w.title);
                self.dirty = true;
            }
            return;
        }
        if code == keymap::KEY_F11 {
            if let Some(top) = self.windows.len().checked_sub(1) {
                self.maximize_toggle(top);
            }
            return;
        }
        if code == keymap::KEY_F5 {
            if let Some(win) = self.windows.last_mut() {
                if matches!(win.app, App::Browser(_)) {
                    browser::reload(win);
                    self.dirty = true;
                }
            }
            return;
        }
        if self.ctrl {
            match code {
                keymap::KEY_F => {
                    if let Some(win) = self.windows.last_mut() {
                        if matches!(win.app, App::Browser(_)) {
                            browser::find_toggle(win);
                            self.dirty = true;
                        }
                    }
                    return;
                }
                keymap::KEY_C | keymap::KEY_A => {
                    self.clipboard_copy();
                    return;
                }
                keymap::KEY_V => {
                    if self.shift {
                        self.open_clipboard_menu(); // Ctrl+Shift+V history picker
                    } else {
                        self.clipboard_paste();
                    }
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
                    // Find bar (Ctrl+F) takes input first, then address bar /
                    // focused form field text entry.
                    if browser::find_char(win, ch) || browser::char_input(win, ch) {
                        self.dirty = true;
                    }
                }
                App::Calc(_) => {
                    calc::key(win, ch);
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
            if let Some((idx, edge)) = self.resize {
                self.apply_resize(idx, edge, self.mx, self.my);
            } else if let Some((idx, ox, oy)) = self.drag {
                // Dragging a maximized/snapped window un-snaps it to follow the cursor.
                let (ox, oy) = if self.windows[idx].restore.is_some() {
                    let (_, _, rw, rh) = self.windows[idx].restore.take().unwrap();
                    self.resize_window(idx, self.mx - rw as isize / 2, self.my - 10, rw, rh);
                    let no = (rw as isize / 2, 10);
                    self.drag = Some((idx, no.0, no.1));
                    no
                } else {
                    (ox, oy)
                };
                let win = &mut self.windows[idx];
                win.x = self.mx - ox;
                let max_y = self.screen.height as isize - TASKBAR_H as isize - win.frame_h();
                win.y = (self.my - oy).min(max_y).max(0);
            } else if let Some(fd) = self.file_drag.as_mut() {
                if !fd.active && ((self.mx - fd.start.0).abs() > 6 || (self.my - fd.start.1).abs() > 6) {
                    fd.active = true;
                    self.cursor = CursorShape::Hand;
                }
            } else if self.icon_drag.is_some() {
                // The floating icon follows the cursor; just need a recompose
                // (dirty already set above).
            } else if self.buttons & 1 != 0 {
                self.forward_mouse_move();
            }
            self.update_cursor();
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
        if pressed & 2 != 0 {
            self.on_right_down();
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

    /// Which window edges (bitmask) the point is within RESIZE_ZONE of, for the
    /// topmost non-maximized window under it.
    fn edge_at(&self, px: isize, py: isize) -> Option<(usize, u8)> {
        for idx in (0..self.windows.len()).rev() {
            let win = &self.windows[idx];
            if win.minimized || win.restore.is_some() {
                continue;
            }
            let (fx, fy) = (win.x, win.y);
            let (fw, fh) = (win.frame_w(), win.frame_h());
            if px < fx - RESIZE_ZONE || px >= fx + fw + RESIZE_ZONE || py < fy - RESIZE_ZONE
                || py >= fy + fh + RESIZE_ZONE
            {
                continue;
            }
            let mut e = 0u8;
            if (px - fx).abs() <= RESIZE_ZONE {
                e |= E_L;
            }
            if (px - (fx + fw)).abs() <= RESIZE_ZONE {
                e |= E_R;
            }
            if (py - fy).abs() <= RESIZE_ZONE {
                e |= E_T;
            }
            if (py - (fy + fh)).abs() <= RESIZE_ZONE {
                e |= E_B;
            }
            if e != 0 {
                return Some((idx, e));
            }
            if win.contains(px, py) {
                return None; // inside content, not on an edge
            }
        }
        None
    }

    /// Resize/move a window to a new geometry, reallocating its canvas
    /// (preserving the overlapping pixels) and re-rendering the app.
    fn resize_window(&mut self, idx: usize, nx: isize, ny: isize, nw: usize, nh: usize) {
        let nw = nw.max(MIN_CW);
        let nh = nh.max(MIN_CH);
        let win = &mut self.windows[idx];
        if win.cw == nw && win.ch == nh && win.x == nx && win.y == ny {
            return;
        }
        let mut nc = vec![0xff1a_1a1a_u32; nw * nh];
        let (cw, ch) = (win.cw.min(nw), win.ch.min(nh));
        for r in 0..ch {
            nc[r * nw..r * nw + cw].copy_from_slice(&win.canvas[r * win.cw..r * win.cw + cw]);
        }
        win.x = nx;
        win.y = ny;
        win.cw = nw;
        win.ch = nh;
        win.canvas = nc;
        render_window_content(win);
        self.dirty = true;
    }

    /// Live resize from a dragged edge: cursor at (mx, my).
    fn apply_resize(&mut self, idx: usize, edge: u8, mx: isize, my: isize) {
        let win = &self.windows[idx];
        let (mut nx, mut ny) = (win.x, win.y);
        let right = win.x + win.frame_w();
        let bottom = win.y + win.frame_h();
        let (mut nw, mut nh) = (win.cw as isize, win.ch as isize);
        if edge & E_L != 0 {
            nx = mx.min(right - (MIN_CW as isize + 2 * BORDER)).max(0);
            nw = right - nx - 2 * BORDER;
        }
        if edge & E_R != 0 {
            nw = mx - win.x - 2 * BORDER;
        }
        if edge & E_T != 0 {
            ny = my.min(bottom - (MIN_CH as isize + TITLE_H + 2 * BORDER)).max(0);
            nh = bottom - ny - TITLE_H - 2 * BORDER;
        }
        if edge & E_B != 0 {
            nh = my - win.y - TITLE_H - 2 * BORDER;
        }
        self.resize_window(idx, nx, ny, nw.max(MIN_CW as isize) as usize, nh.max(MIN_CH as isize) as usize);
    }

    /// Toggle maximize (fill desktop minus taskbar) / restore.
    fn maximize_toggle(&mut self, idx: usize) {
        if let Some((rx, ry, rw, rh)) = self.windows[idx].restore.take() {
            self.resize_window(idx, rx, ry, rw, rh);
            kprintln!("WM: restored '{}'", self.windows[idx].title);
        } else {
            let win = &self.windows[idx];
            self.windows[idx].restore = Some((win.x, win.y, win.cw, win.ch));
            let nw = self.screen.width - 2 * BORDER as usize;
            let nh = self.screen.height - TASKBAR_H - TITLE_H as usize - 2 * BORDER as usize;
            self.resize_window(idx, 0, 0, nw, nh);
            kprintln!("WM: maximized '{}'", self.windows[idx].title);
        }
    }

    fn minimize(&mut self, idx: usize) {
        self.windows[idx].minimized = true;
        kprintln!("WM: minimized '{}'", self.windows[idx].title);
        self.dirty = true;
    }

    /// Snap a dragged window to a screen-edge zone (left/right half, top=max).
    fn try_snap(&mut self, idx: usize) {
        let (w, h) = (self.screen.width as isize, self.screen.height as isize);
        let avail_h = h - TASKBAR_H as isize;
        const SNAP: isize = 6;
        let win = &self.windows[idx];
        if win.restore.is_some() {
            return;
        }
        let cur = (win.x, win.y, win.cw, win.ch);
        if self.my <= SNAP {
            self.windows[idx].restore = Some(cur);
            self.maximize_toggle_from(idx);
        } else if self.mx <= SNAP {
            self.windows[idx].restore = Some(cur);
            let hw = (w / 2 - 2 * BORDER) as usize;
            self.resize_window(idx, 0, 0, hw, (avail_h - TITLE_H - 2 * BORDER) as usize);
            kprintln!("WM: snapped left '{}'", self.windows[idx].title);
        } else if self.mx >= w - SNAP {
            self.windows[idx].restore = Some(cur);
            let hw = (w / 2 - 2 * BORDER) as usize;
            self.resize_window(idx, w / 2, 0, hw, (avail_h - TITLE_H - 2 * BORDER) as usize);
            kprintln!("WM: snapped right '{}'", self.windows[idx].title);
        }
    }

    fn maximize_toggle_from(&mut self, idx: usize) {
        // restore already saved by caller; fill the desktop.
        let nw = self.screen.width - 2 * BORDER as usize;
        let nh = self.screen.height - TASKBAR_H - TITLE_H as usize - 2 * BORDER as usize;
        self.resize_window(idx, 0, 0, nw, nh);
        kprintln!("WM: snapped top->max '{}'", self.windows[idx].title);
    }

    /// Recompute the pointer shape from what's under the cursor.
    fn update_cursor(&mut self) {
        let cur = if self.resize.is_some() {
            edge_cursor(self.resize.unwrap().1)
        } else if self.my >= self.screen.height as isize - TASKBAR_H as isize {
            CursorShape::Arrow
        } else if let Some((_, e)) = self.edge_at(self.mx, self.my) {
            edge_cursor(e)
        } else {
            self.content_cursor()
        };
        if cur != self.cursor {
            self.cursor = cur;
            self.dirty = true;
        }
    }

    fn content_cursor(&self) -> CursorShape {
        if let Some((idx, Hit::Content(rx, ry))) = self.hit_test(self.mx, self.my) {
            let win = &self.windows[idx];
            return match &win.app {
                App::Browser(_) => {
                    if ry >= browser::TOPBAR as isize && browser::link_at(win, rx, ry).is_some() {
                        CursorShape::Hand
                    } else {
                        CursorShape::IBeam
                    }
                }
                App::Editor(_) | App::Shell { .. } | App::Lisp(_) | App::Echo { .. } => CursorShape::IBeam,
                _ => CursorShape::Arrow,
            };
        }
        CursorShape::Arrow
    }

    /// Ctrl+Shift+V: a popup of the last clipboard entries; click one to paste.
    fn open_clipboard_menu(&mut self) {
        let hist = crate::clipboard::history();
        if hist.is_empty() {
            self.notify("Clipboard history empty");
            return;
        }
        let items: Vec<String> = hist
            .iter()
            .map(|s| s.chars().take(22).collect::<String>().replace('\n', " "))
            .collect();
        let mh = items.len() as isize * 26 + 8;
        self.menu = Some(ContextMenu {
            items,
            x: self.mx.min(self.screen.width as isize - 180).max(0),
            y: (self.my - mh).max(0),
            target: MenuTarget::Clipboard,
        });
        self.dirty = true;
    }

    fn on_right_down(&mut self) {
        if self.menu.take().is_some() {
            self.dirty = true;
            return;
        }
        if self.my >= self.screen.height as isize - TASKBAR_H as isize {
            return;
        }
        let win_items = || ["Minimize", "Maximize", "Close"].iter().map(|s| s.to_string()).collect();
        let (items, target): (Vec<String>, MenuTarget) = match self.hit_test(self.mx, self.my) {
            Some((idx, Hit::Title)) => {
                let top = self.raise(idx);
                (win_items(), MenuTarget::Window(top))
            }
            Some((idx, Hit::Content(..))) => {
                let top = self.raise(idx);
                (win_items(), MenuTarget::Window(top))
            }
            None => (
                ["New File", "New Folder", "Screenshot", "Change Wallpaper", "Settings"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                MenuTarget::Desktop,
            ),
        };
        let (mw, mh) = (168isize, items.len() as isize * 26 + 8);
        let x = self.mx.min(self.screen.width as isize - mw - 2).max(0);
        let y = self.my.min(self.screen.height as isize - mh - TASKBAR_H as isize - 2).max(0);
        self.menu = Some(ContextMenu { items, x, y, target });
        self.dirty = true;
    }

    /// Handle a left click while a menu is open. Returns true if absorbed.
    fn menu_click(&mut self) -> bool {
        let Some(menu) = self.menu.take() else { return false };
        self.dirty = true;
        let mw = 168isize;
        let inside = self.mx >= menu.x && self.mx < menu.x + mw && self.my >= menu.y + 4;
        let row = ((self.my - menu.y - 4) / 26) as usize;
        if !inside || row >= menu.items.len() {
            return true; // click outside an item just closes the menu
        }
        let item = menu.items[row].as_str();
        if let MenuTarget::Clipboard = menu.target {
            crate::clipboard::pick(row);
            self.clipboard_paste();
            return true;
        }
        match (&menu.target, item) {
            (MenuTarget::Window(idx), "Minimize") => self.minimize(*idx),
            (MenuTarget::Window(idx), "Maximize") => self.maximize_toggle(*idx),
            (MenuTarget::Window(idx), "Close") => {
                if *idx < self.windows.len() {
                    let w = self.windows.remove(*idx);
                    kprintln!("WM: closed '{}'", w.title);
                }
            }
            (MenuTarget::Desktop, "Screenshot") => self.screenshot(false),
            (MenuTarget::Desktop, "New File") => {
                if crate::fs::mounted() && crate::fs::write_file("UNTITLED.TXT", b"").is_ok() {
                    self.notify("Created UNTITLED.TXT");
                }
            }
            (MenuTarget::Desktop, "New Folder") => self.notify("Folders: use the file manager"),
            (MenuTarget::Desktop, "Change Wallpaper") => {
                set_wallpaper_next();
                self.notify("Wallpaper changed");
            }
            (MenuTarget::Desktop, "Settings") => self.launch("settings"),
            _ => {}
        }
        true
    }

    fn on_left_down(&mut self) {
        kprintln!("CLICK: left down @ ({}, {})", self.mx, self.my);
        if self.menu_click() {
            return;
        }
        if self.my >= self.screen.height as isize - TASKBAR_H as isize {
            self.taskbar_click(self.mx);
            return;
        }
        // Resize edge takes priority over the (overlapping) title/content hit.
        if let Some((idx, edge)) = self.edge_at(self.mx, self.my) {
            let top = self.raise(idx);
            self.resize = Some((top, edge));
            return;
        }
        match self.hit_test(self.mx, self.my) {
            Some((idx, Hit::Title)) => {
                let top = self.raise(idx);
                // Title-bar buttons (from the right edge): close, max, min.
                let rx = self.mx - self.windows[top].x - BORDER;
                let cw = self.windows[top].cw as isize;
                if rx >= cw - TBTN_W {
                    let win = self.windows.remove(top);
                    kprintln!("WM: closed '{}'", win.title);
                    self.dirty = true;
                    return;
                } else if rx >= cw - 2 * TBTN_W {
                    self.maximize_toggle(top);
                    return;
                } else if rx >= cw - 3 * TBTN_W {
                    self.minimize(top);
                    return;
                }
                // Double-click the title bar (not on a button) -> maximize.
                let now = timer::ticks();
                let (lt, lx, ly) = self.last_title_click;
                if now.wrapping_sub(lt) < 30 && (self.mx - lx).abs() < 6 && (self.my - ly).abs() < 6 {
                    self.last_title_click = (0, 0, 0);
                    self.maximize_toggle(top);
                    return;
                }
                self.last_title_click = (now, self.mx, self.my);
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
                    App::Files(_) => {
                        // Record a potential drag-out; selection happens now,
                        // the open is deferred to release (so a press-move-drop
                        // can route the file to another app).
                        let name = files::name_at(win, rx, ry);
                        files::click(win, rx, ry); // updates selection / redraw
                        self.dirty = true;
                        if let Some(name) = name {
                            self.file_drag = Some(FileDrag { name, start: (self.mx, self.my), active: false });
                        }
                    }
                    App::Gif(_) => {
                        gifplayer::click(win);
                        self.dirty = true;
                    }
                    App::Calc(_) => {
                        calc::click(win, rx, ry);
                        self.dirty = true;
                    }
                    App::Settings(_) => {
                        if settings::click(win, rx, ry) {
                            scheduler::spawn_kernel("audio", snd::audio_task);
                        }
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

    /// Pill geometry for the taskbar: (app, label, x, width). Pill widths are
    /// the label width + padding, compressed proportionally if the row would
    /// otherwise overrun the clock — so any number of apps fits the screen.
    /// Render and hit-test both call this so they can never disagree.
    fn taskbar_layout(&self) -> Vec<(&'static str, &'static str, usize, usize)> {
        let sw = self.screen.width;
        let apps = launchers();
        if apps.is_empty() {
            return Vec::new();
        }
        const START: usize = 58; // just past the VEIL wordmark
        const CLOCK_SPACE: usize = 60; // reserve room for the clock on the right
        const GAP: usize = 4;
        let small = font::ui_small();
        let nat: Vec<usize> = apps.iter().map(|(_, l)| font::text_width(small, l) + 16).collect();
        let total: usize = nat.iter().sum();
        let avail = sw.saturating_sub(START + CLOCK_SPACE + GAP * apps.len());
        let mut out = Vec::with_capacity(apps.len());
        let mut x = START;
        for (i, (app, label)) in apps.iter().enumerate() {
            let wdt = if total > avail && total > 0 { (nat[i] * avail / total).max(14) } else { nat[i] };
            out.push((*app, *label, x, wdt));
            x += wdt + GAP;
        }
        out
    }

    fn taskbar_click(&mut self, px: isize) {
        for (app, _, x, wdt) in self.taskbar_layout() {
            if px >= x as isize && px < (x + wdt) as isize {
                kprintln!("WM: taskbar -> '{app}'");
                // Existing window: restore if minimized, minimize if focused,
                // else raise. Otherwise launch it fresh.
                if let Some(idx) = self.windows.iter().position(|w| w.title == app) {
                    if self.windows[idx].minimized {
                        self.windows[idx].minimized = false;
                        self.raise(idx);
                        kprintln!("WM: restored '{app}' from taskbar");
                    } else if idx == self.windows.len() - 1 {
                        self.minimize(idx);
                    } else {
                        self.raise(idx);
                    }
                } else {
                    self.launch(app);
                }
                return;
            }
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
        // Finish a file drag-and-drop (or a plain click in the file manager).
        if let Some(fd) = self.file_drag.take() {
            if fd.active {
                // Dropped somewhere: route to the window under the cursor.
                let target = self
                    .hit_test(self.mx, self.my)
                    .filter(|(idx, _)| !matches!(self.windows[*idx].app, App::Files(_)))
                    .map(|(idx, _)| self.windows[idx].title.clone());
                kprintln!("WM: dropped '{}' on {}", fd.name, target.as_deref().unwrap_or("desktop"));
                self.open_file(&fd.name);
                self.notify(&format!("Opened {}", fd.name));
            } else {
                // A plain click opens the file in its default app.
                self.open_file(&fd.name);
            }
            self.dirty = true;
            return;
        }
        // Finish an active resize.
        if let Some((idx, _)) = self.resize.take() {
            let win = &self.windows[idx];
            kprintln!("WM: resized '{}' to {}x{}", win.title, win.cw, win.ch);
            self.update_cursor();
            return;
        }
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
            self.try_snap(idx); // drag-to-edge snapping
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
        let before = self.toasts.len();
        self.toasts.retain(|(_, exp)| now < *exp);
        if self.toasts.len() != before {
            self.dirty = true;
        }
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
        WINDOW_COUNT.store(self.windows.len(), core::sync::atomic::Ordering::Relaxed);
        back.clear(DESKTOP_BG);
        if let Some(wp) = wallpaper() {
            // Scale the decoded wallpaper to fill the desktop (nearest-neighbour).
            for y in 0..h {
                let sy = (y * wp.h / h).min(wp.h - 1);
                for x in 0..w {
                    let sx = (x * wp.w / w).min(wp.w - 1);
                    back.put_pixel(x, y, wp.pixels[sy * wp.w + sx]);
                }
            }
            let _ = DESKTOP_GRID;
        } else {
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
            ui_centered(&back, cx + iw / 2, cy + 7, icon_abbrev(app), FontId::UiBold, 19, 0xfff4f4f4);
            // Full app name below.
            ui_centered(&back, cx + iw / 2, cy + iw + 1, icon_label(app), FontId::Ui, 12, 0xffc8ccd4);
        }

        let top = self.windows.len().saturating_sub(1);
        for (idx, win) in self.windows.iter().enumerate() {
            if win.minimized {
                continue; // minimized windows stay alive but unpainted
            }
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
                back.draw_text(txu + 10, tyu + 3, &win.title, FontId::Ui, 13, tcol);
                // Window buttons (macOS traffic-light style): minimize (amber),
                // maximize (green), close (red), each in a TBTN_W slot from right.
                let ccy = ty + TITLE_H / 2;
                let cx = |slot: isize| tx + win.cw as isize - slot * TBTN_W + TBTN_W / 2;
                let (close_c, max_c, min_c) = if focused {
                    (0xffe0_5555, 0xff5fc26a, 0xffd6a844)
                } else {
                    (0xff55_5555, 0xff444444, 0xff444444)
                };
                back.fill_circle(cx(1), ccy, 6, close_c);
                back.fill_circle(cx(2), ccy, 6, max_c);
                back.fill_circle(cx(3), ccy, 6, min_c);
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
        // Pill widths are dynamic + compressed so any number of apps fits.
        let ty = h - TASKBAR_H;
        back.fill_rect(0, ty, w, TASKBAR_H, 0xff111111);
        back.fill_rect(0, ty, w, 1, 0xff262626); // hairline top edge
        ui_text(&back, 12, ty + 7, "VEIL", FontId::UiBold, 13, ACCENT);
        let layout = self.taskbar_layout();
        if !TASKBAR_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
            for (app, _, x, pw) in &layout {
                kprintln!("TASKBAR_PILL: {app} {x} {pw}");
            }
        }
        for (app, label, x, pw) in &layout {
            let win = self.windows.iter().find(|win| win.title == *app);
            let (pill, txt) = match win {
                Some(w) if w.minimized => (blend(0xff111111, ACCENT, 30), blend(ACCENT, 0xff111111, 100)),
                Some(_) => (blend(0xff111111, ACCENT, 70), ACCENT),
                None => (0xff1e1e1e, 0xff8a8a8a),
            };
            back.fill_round_rect(*x, ty + 5, *pw, TASKBAR_H - 10, 7, pill);
            ui_centered(&back, x + pw / 2, ty + 8, &fit_label(&back, label, pw.saturating_sub(8), FontId::Ui, 12), FontId::Ui, 12, txt);
        }
        // Clock on the far right.
        let clk = clock_string();
        let (cw_px, _) = back.measure_text(&clk, FontId::Ui, 12);
        back.draw_text(w - cw_px - 12, ty + 8, &clk, FontId::Ui, 12, 0xffb0b0b0);

        // The dragged icon floats semi-transparent under the cursor.
        if let Some(slot) = self.icon_drag {
            if let Some(app) = self.icon_order.get(slot) {
                let iw = ICON_W as usize;
                let fx = (self.mx - ICON_W / 2).max(0) as usize;
                let fy = (self.my - ICON_W / 2).max(0) as usize;
                back.blend_rect(fx, fy, iw, iw, icon_color(*app), 150);
                ui_centered(&back, fx + iw / 2, fy + 7, icon_abbrev(app), FontId::UiBold, 19, 0xfff4f4f4);
            }
        }

        // Drag-and-drop ghost: a semi-transparent filename chip under the cursor.
        if let Some(fd) = &self.file_drag {
            if fd.active {
                let gw = back.measure_text(&fd.name, FontId::Ui, 13).0 + 20;
                let gx = (self.mx + 10).max(0) as usize;
                let gy = (self.my + 6).max(0) as usize;
                back.blend_rect(gx, gy, gw, 22, ACCENT, 170);
                back.draw_text(gx + 10, gy + 3, &fd.name, FontId::Ui, 13, 0xffffffff);
            }
        }

        // Right-click context menu.
        if let Some(menu) = &self.menu {
            let mw = 168usize;
            let mh = menu.items.len() * 26 + 8;
            let mx = (menu.x as usize).min(w.saturating_sub(mw + 2));
            let my = (menu.y as usize).min(h.saturating_sub(mh + TASKBAR_H + 2));
            back.blend_rect(mx + 3, my + 3, mw, mh, 0xff00_0000, 90);
            back.fill_round_rect(mx, my, mw, mh, 8, 0xff24_2424);
            back.fill_round_rect(mx, my, mw, mh, 8, 0xff24_2424);
            for (i, item) in menu.items.iter().enumerate() {
                let iy = my + 4 + i * 26;
                let hover = self.my >= iy as isize && self.my < (iy + 26) as isize
                    && self.mx >= mx as isize && self.mx < (mx + mw) as isize;
                if hover {
                    back.fill_round_rect(mx + 4, iy, mw - 8, 24, 5, ACCENT);
                }
                back.draw_text(mx + 14, iy + 4, item, FontId::Ui, 14, if hover { 0xff0d0d0d } else { 0xffe0e0e0 });
            }
        }

        // Toast notifications, stacked in the bottom-right above the taskbar.
        for (i, (msg, _)) in self.toasts.iter().rev().enumerate() {
            let tw = (back.measure_text(msg, FontId::Ui, 14).0 + 28).min(w - 20);
            let th = 34usize;
            let tx = w - tw - 14;
            let ty = h - TASKBAR_H - (i + 1) * (th + 8);
            back.blend_rect(tx + 3, ty + 3, tw, th, 0xff00_0000, 90);
            back.fill_round_rect(tx, ty, tw, th, 8, 0xff2b_2b2b);
            back.fill_round_rect(tx, ty, 4, th, 2, ACCENT);
            back.draw_text(tx + 14, ty + 8, msg, FontId::Ui, 14, 0xffe8e8e8);
        }

        // Screenshot flash.
        if timer::ticks() < self.flash {
            back.blend_rect(0, 0, w, h, 0xffff_ffff, 160);
        }

        // Cursor, always on top, shape depends on what is under it.
        draw_cursor(&back, self.mx, self.my, self.cursor);

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
    const SH: u16 = 14; // JetBrains Mono px size
    let lh = 17usize;
    let (input, visible) = {
        let App::Shell { input, lines, .. } = &win.app else { return };
        let rows = win.ch.saturating_sub(lh + 8) / lh;
        let skip = lines.len().saturating_sub(rows);
        (input.clone(), lines[skip..].to_vec())
    };
    let fb = win.canvas_fb();
    fb.clear(0xff14_1414);
    let chev = fb.measure_text("> ", FontId::Mono, SH).0;
    let is_err = |s: &str| s.contains("not found") || s.contains("no such") || s.contains("failed") || s.contains(": error");
    let mut y = 6;
    for line in &visible {
        let text = line.trim_end_matches('\n');
        if let Some(cmd) = text.strip_prefix("> ") {
            fb.draw_text(6, y, ">", FontId::Mono, SH, ACCENT); // chevron prompt
            fb.draw_text(6 + chev, y, cmd, FontId::Mono, SH, 0xffff_ffff);
        } else {
            fb.draw_text(6, y, text, FontId::Mono, SH, if is_err(text) { 0xffe0_5555 } else { 0xffcc_cccc });
        }
        y += lh;
    }
    let py = win.ch.saturating_sub(lh + 2);
    fb.draw_text(6, py, ">", FontId::Mono, SH, ACCENT);
    fb.draw_text(6 + chev, py, &format!("{input}_"), FontId::Mono, SH, 0xffff_ffff);
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
static TASKBAR_LOGGED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
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

// Dark code-editor theme (VS Code-ish).
const ED_BG: u32 = 0xff1e_1e1e;
const ED_GUTTER: u32 = 0xff25_2526;
const ED_LINENO: u32 = 0xff85_8585;
const ED_FG: u32 = 0xffd4_d4d4;
const ED_KW: u32 = 0xff56_9cd6;
const ED_STR: u32 = 0xffce_9178;
const ED_NUM: u32 = 0xffb5_cea8;
const ED_COMMENT: u32 = 0xff6a_9955;

fn editor_lang(file: &str) -> &'static str {
    let f = file.to_ascii_uppercase();
    if f.ends_with(".RS") {
        "rust"
    } else if f.ends_with(".PY") {
        "py"
    } else if f.ends_with(".JS") {
        "js"
    } else if f.ends_with(".HTM") || f.ends_with(".HTML") {
        "html"
    } else if f.ends_with(".CSS") {
        "css"
    } else if f.ends_with(".SH") {
        "sh"
    } else {
        "text"
    }
}

fn is_keyword(word: &str, lang: &str) -> bool {
    const RUST: &[&str] = &["fn", "let", "mut", "pub", "struct", "enum", "impl", "for", "while", "loop",
        "if", "else", "match", "return", "use", "mod", "const", "static", "self", "Self", "trait", "where",
        "as", "in", "ref", "move", "unsafe", "async", "await", "type", "dyn", "crate", "super", "true", "false"];
    const PY: &[&str] = &["def", "class", "import", "from", "if", "else", "elif", "for", "while", "return",
        "self", "None", "True", "False", "and", "or", "not", "in", "is", "lambda", "with", "as", "try", "except", "pass", "yield"];
    const JS: &[&str] = &["function", "var", "let", "const", "if", "else", "for", "while", "return", "class",
        "new", "this", "typeof", "instanceof", "null", "undefined", "true", "false", "async", "await", "import", "export"];
    const SH: &[&str] = &["if", "then", "else", "fi", "for", "while", "do", "done", "case", "esac", "echo",
        "export", "return", "function", "in"];
    let set: &[&str] = match lang {
        "rust" => RUST,
        "py" => PY,
        "js" | "html" | "css" => JS,
        "sh" => SH,
        _ => &[],
    };
    set.contains(&word)
}

/// Tokenise a source line into coloured spans for syntax highlighting.
fn highlight_line(line: &str, lang: &str) -> Vec<(String, u32)> {
    if lang == "text" {
        return alloc::vec![(line.to_string(), ED_FG)];
    }
    let comment = if lang == "py" || lang == "sh" { "#" } else { "//" };
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // line comment to end
        if line[byte_at(line, i)..].starts_with(comment) {
            spans.push((chars[i..].iter().collect(), ED_COMMENT));
            break;
        }
        if c == '"' || c == '\'' {
            let q = c;
            let mut s = String::new();
            s.push(c);
            i += 1;
            while i < chars.len() {
                s.push(chars[i]);
                if chars[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((s, ED_STR));
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                s.push(chars[i]);
                i += 1;
            }
            spans.push((s, ED_NUM));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                i += 1;
            }
            let col = if is_keyword(&s, lang) { ED_KW } else { ED_FG };
            spans.push((s, col));
            continue;
        }
        spans.push((c.to_string(), ED_FG));
        i += 1;
    }
    spans
}

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}

fn render_editor(win: &mut Window) {
    let (file, status, text) = {
        let App::Editor(st) = &win.app else { return };
        (st.file.clone(), st.status.clone(), st.text.clone())
    };
    let lang = editor_lang(&file);
    let (cw, ch) = (win.cw, win.ch);
    const LH: usize = 17;
    const GUT_W: usize = 44;
    let top = TOOLBAR_H as usize;
    let rows = (ch - top - 4) / LH;

    // Split on hard newlines only (no soft wrap with proportional metrics).
    let src_lines: Vec<&str> = if text.is_empty() { alloc::vec![""] } else { text.split('\n').collect() };
    let nlines = src_lines.len();
    let cur_line = nlines; // cursor is at the end (1-based last line)
    let cur_col = src_lines.last().map(|l| l.chars().count()).unwrap_or(0) + 1;
    let total_rows = src_lines.len();
    let scroll = total_rows.saturating_sub(rows); // keep the cursor (end) in view

    let fb = win.canvas_fb();
    fb.fill_rect(0, top, cw, ch - top, ED_BG);
    fb.fill_rect(0, top, GUT_W, ch - top, ED_GUTTER);
    render_editor_toolbar(&fb, cw, &file, &status);
    for (r, line) in src_lines.iter().skip(scroll).take(rows).enumerate() {
        let y = top + 2 + r * LH;
        let lineno = scroll + r + 1;
        let num = format!("{lineno}");
        let (nw, _) = fb.measure_text(&num, FontId::Mono, 13);
        fb.draw_text(GUT_W - nw - 6, y + 1, &num, FontId::Mono, 13, ED_LINENO);
        let mut x = GUT_W + 6;
        for (span, col) in highlight_line(line, lang) {
            fb.draw_text(x, y, &span, FontId::Mono, 14, col);
            x += fb.measure_text(&span, FontId::Mono, 14).0;
        }
    }
    // Cursor at the end of the buffer.
    if cur_line > scroll && cur_line - scroll <= rows {
        let y = top + 2 + (cur_line - 1 - scroll) * LH;
        let cx = GUT_W + 6 + fb.measure_text(src_lines.last().unwrap_or(&""), FontId::Mono, 14).0;
        fb.fill_rect(cx, y, 2, LH - 1, ED_FG);
    }
    // Status bar: line/col + size.
    let sy = ch - 18;
    fb.fill_rect(0, sy, cw, 18, 0xff007a_cc & 0x00ff_ffff | 0xff00_0000);
    let info = format!("Ln {cur_line}, Col {cur_col}    {} bytes    {}", text.len(), lang);
    fb.draw_text(8, sy + 2, &info, FontId::Ui, 12, 0xffffffff);
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
