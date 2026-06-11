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

    fn clamp_scroll(&mut self, rows: usize) {
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if rows > 0 && self.sel >= self.scroll + rows {
            self.scroll = self.sel + 1 - rows;
        }
    }
}

fn tag(name: &str) -> &'static str {
    if name.ends_with(".PNG") {
        "[IMG]"
    } else if name.ends_with(".WAV") {
        "[WAV]"
    } else if name.ends_with(".TXT") {
        "[TXT]"
    } else {
        "[???]"
    }
}

/// True for names the file manager knows how to launch.
fn openable(name: &str) -> bool {
    name.ends_with(".PNG") || name.ends_with(".WAV") || name.ends_with(".TXT")
}

pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let (files, sel, scroll) = {
        let App::Files(st) = &win.app else { return };
        (st.files.clone(), st.sel, st.scroll)
    };
    let rows = ch / ROW_H;
    let fb = win.canvas_fb();
    fb.clear(BG);
    for r in 0..rows {
        let i = scroll + r;
        if i >= files.len() {
            break;
        }
        let y = r * ROW_H;
        let selected = i == sel;
        if selected {
            fb.fill_rect(0, y, cw, ROW_H, SEL_BG);
        }
        let (tcol, ncol) = if selected { (SEL_TX, SEL_TX) } else { (TAG_COL, TEXT) };
        fb.draw_string(4, y, tag(&files[i]), tcol, None);
        fb.draw_string(4 + 6 * 8, y, &files[i], ncol, None);
    }
}

/// Keyboard: up/down move the selection, Enter opens it. Returns the action
/// for the WM (it performs Open, since launching touches other windows).
pub fn key(win: &mut Window, code: u16) -> Action {
    let rows = win.ch / ROW_H;
    let App::Files(st) = &mut win.app else { return Action::None };
    match code {
        KEY_UP => {
            st.sel = st.sel.saturating_sub(1);
            st.clamp_scroll(rows);
            Action::Redraw
        }
        KEY_DOWN => {
            if st.sel + 1 < st.files.len() {
                st.sel += 1;
            }
            st.clamp_scroll(rows);
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
pub fn click(win: &mut Window, _rx: isize, ry: isize) -> Action {
    if ry < 0 {
        return Action::None;
    }
    let App::Files(st) = &mut win.app else { return Action::None };
    let i = st.scroll + ry as usize / ROW_H;
    let Some(name) = st.files.get(i).cloned() else { return Action::None };
    st.sel = i;
    if openable(&name) {
        Action::Open(name)
    } else {
        Action::Redraw
    }
}
