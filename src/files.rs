//! M29: in-OS file manager (App::Files). Lists every file on the FAT16
//! disk, one per row with a bitmap-font type tag, and opens each in the
//! right app: PNG -> Viewer, WAV -> Audio, TXT -> Editor. Read-only — no
//! delete/rename/write. Scrolls with the up/down arrows or the mouse wheel.

use crate::fb::Framebuffer;
use crate::wm::{App, Window};
use crate::{fs, kprintln};
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff14_1a22;
const TEXT: u32 = 0xffd0_d8e0;
const TAG_COL: u32 = 0xff7a_b0e0;
const SEL_BG: u32 = 0xff2a_5a8a;
const SEL_TX: u32 = 0xffff_ffff;
pub const ROW_H: usize = 14;

const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_ENTER: u16 = 28;

pub struct FilesState {
    files: Vec<String>,
    sel: usize,
    scroll: usize,
}

/// What a key/click resolved to.
pub enum Action {
    None,
    Redraw,
    Open(String),
}

impl FilesState {
    pub fn new() -> FilesState {
        let mut files: Vec<String> =
            fs::list_root().unwrap_or_default().into_iter().map(|(n, _)| n).collect();
        files.sort();
        kprintln!("FILES: {} files on disk", files.len());
        for (i, f) in files.iter().enumerate() {
            kprintln!("FILES[{i}]: {f}");
        }
        FilesState { files, sel: 0, scroll: 0 }
    }

    fn clamp_scroll(&mut self, page: usize) {
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if page > 0 && self.sel >= self.scroll + page {
            self.scroll = self.sel + 1 - page;
        }
    }
}

fn page_size(cw: usize, ch: usize) -> usize {
    let rows = (ch / ROW_H).max(1);
    let col_w = 4 + 48 + 4 + 12 * 8 + 8;
    let cols = (cw / col_w).max(1);
    rows * cols
}

fn is_image(name: &str) -> bool {
    name.ends_with(".PNG") || name.ends_with(".JPG") || name.ends_with(".JPEG")
}

fn tag(name: &str) -> &'static str {
    if is_image(name) {
        "[IMG]"
    } else if name.ends_with(".GIF") {
        "[GIF]"
    } else if name.ends_with(".WAV") {
        "[WAV]"
    } else if name.ends_with(".TXT") {
        "[TXT]"
    } else if name.ends_with(".MJPEG") || name.ends_with(".AVI") || name.ends_with(".MJPG") {
        "[VID]"
    } else if name.ends_with(".WASM") {
        "[WSM]"
    } else {
        "[???]"
    }
}

/// True for names the file manager knows how to launch.
fn openable(name: &str) -> bool {
    is_image(name)
        || name.ends_with(".GIF")
        || name.ends_with(".WAV")
        || name.ends_with(".TXT")
        || name.ends_with(".MJPEG")
        || name.ends_with(".MJPG")
        || name.ends_with(".AVI")
        || name.ends_with(".WASM")
}

pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let (files, sel, scroll) = {
        let App::Files(st) = &win.app else { return };
        (st.files.clone(), st.sel, st.scroll)
    };
    let rows = (ch / ROW_H).max(1);
    // Column width: tag (6 chars * 8px = 48) + space + up to 12 chars filename + padding.
    let col_w = 4 + 48 + 4 + 12 * 8 + 8; // ~164px
    let cols = (cw / col_w).max(1);
    let fb = win.canvas_fb();
    fb.clear(BG);
    for slot in 0..(rows * cols) {
        let i = scroll + slot;
        if i >= files.len() {
            break;
        }
        let col = slot / rows;
        let row = slot % rows;
        let x = col * col_w;
        let y = row * ROW_H;
        // Skip any slot whose row/column origin lands outside the canvas, so a
        // wide listing never starts a blit past the window edge.
        if x >= cw || y >= ch {
            continue;
        }
        let selected = i == sel;
        if selected {
            fb.fill_rect(x, y, col_w, ROW_H, SEL_BG);
        }
        let (tcol, ncol) = if selected { (SEL_TX, SEL_TX) } else { (TAG_COL, TEXT) };
        fb.draw_string(x + 4, y, tag(&files[i]), tcol, None);
        // Truncate filename to fit column.
        let name = &files[i];
        let max_chars = ((col_w - 4 - 48 - 4) / 8).min(name.len());
        fb.draw_string(x + 4 + 48 + 4, y, &name[..max_chars], ncol, None);
    }
}

/// Keyboard: up/down move the selection, Enter opens it. Returns the action
/// for the WM (it performs Open, since launching touches other windows).
pub fn key(win: &mut Window, code: u16) -> Action {
    let page = page_size(win.cw, win.ch);
    let App::Files(st) = &mut win.app else { return Action::None };
    match code {
        KEY_UP => {
            st.sel = st.sel.saturating_sub(1);
            st.clamp_scroll(page);
            Action::Redraw
        }
        KEY_DOWN => {
            if st.sel + 1 < st.files.len() {
                st.sel += 1;
            }
            st.clamp_scroll(page);
            Action::Redraw
        }
        KEY_ENTER => match st.files.get(st.sel) {
            Some(name) if openable(name) => Action::Open(name.clone()),
            _ => Action::None,
        },
        _ => Action::None,
    }
}

/// Click a row: select it and, if it's openable, dispatch it.
pub fn click(win: &mut Window, rx: isize, ry: isize) -> Action {
    if ry < 0 || rx < 0 {
        return Action::None;
    }
    let col_w = 4 + 48 + 4 + 12 * 8 + 8;
    let rows = (win.ch / ROW_H).max(1);
    let col = rx as usize / col_w;
    let row = ry as usize / ROW_H;
    let App::Files(st) = &mut win.app else { return Action::None };
    let slot = col * rows + row;
    let i = st.scroll + slot;
    let Some(name) = st.files.get(i).cloned() else { return Action::None };
    st.sel = i;
    if openable(&name) {
        Action::Open(name)
    } else {
        Action::Redraw
    }
}
