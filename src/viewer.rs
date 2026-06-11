//! M23: the image viewer app. Opens every `.PNG` on the FAT16 disk
//! (alphabetical), decoding with the kernel's own `png` module, and shows
//! one at a time scaled to fit the window with nearest-neighbour sampling.
//! Left/right arrows cycle through the images; the window title is the
//! current filename. Images smaller than the window are centred on a flat
//! background; larger ones are shrunk to fit, aspect preserved.

use crate::wm::Window;
use crate::{fs, kprintln, png};
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff14_181c; // letterbox / background fill behind the image

pub struct ViewerState {
    files: Vec<String>,      // sorted .PNG names on the disk
    idx: usize,              // current image
    img: Option<png::Image>, // decoded current image (None = undecodable)
}

impl ViewerState {
    pub fn new() -> ViewerState {
        let mut files: Vec<String> = fs::list_root()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|n| n.ends_with(".PNG")) // FAT 8.3 names are upper-case
            .collect();
        files.sort();
        let mut st = ViewerState { files, idx: 0, img: None };
        st.load();
        st
    }

    /// M29: open the viewer positioned on a specific file (chosen in the
    /// file manager). Falls back to the first image if the name isn't found.
    pub fn with_file(name: &str) -> ViewerState {
        let mut st = ViewerState::new();
        if let Some(i) = st.files.iter().position(|f| f == name) {
            st.idx = i;
            st.load();
        }
        st
    }

    /// Decode the current file and log it (the proof greps these lines).
    fn load(&mut self) {
        self.img = None;
        let Some(name) = self.files.get(self.idx) else {
            kprintln!("VIEWER: no .PNG files on disk");
            return;
        };
        match fs::read_file(name).and_then(|d| png::decode(&d)) {
            Some(im) => {
                kprintln!("VIEWER: showing {name} {}x{}", im.w, im.h);
                self.img = Some(im);
            }
            None => kprintln!("VIEWER: cannot decode {name}"),
        }
    }

    /// Filename for the window title bar.
    pub fn current_name(&self) -> String {
        self.files
            .get(self.idx)
            .cloned()
            .unwrap_or_else(|| String::from("no images"))
    }

    fn step(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as isize;
        self.idx = (((self.idx as isize + delta) % n + n) % n) as usize;
        self.load();
    }
}

/// Left/right arrow cycles images. Returns true if the key was consumed
/// (and updates the window title to the new filename).
pub fn key(win: &mut Window, code: u16) -> bool {
    const KEY_LEFT: u16 = 105;
    const KEY_RIGHT: u16 = 106;
    let delta = match code {
        KEY_LEFT => -1,
        KEY_RIGHT => 1,
        _ => return false,
    };
    {
        let crate::wm::App::Viewer(st) = &mut win.app else { return false };
        st.step(delta);
        win.title = st.current_name();
    }
    render(win);
    true
}

/// Draw the current image into the window canvas: aspect-preserving
/// nearest-neighbour fit, centred on the background.
pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    let crate::wm::App::Viewer(st) = &win.app else { return };
    let Some(im) = &st.img else {
        let msg = if st.files.is_empty() { "no .PNG files on disk" } else { "cannot decode image" };
        fb.draw_string(8, 8, msg, 0xffd0_d8e0, None);
        return;
    };
    if im.w == 0 || im.h == 0 {
        return;
    }
    // Fit to the content box, preserving aspect (scale in 1/1024 units).
    let scale = ((cw * 1024 / im.w).min(ch * 1024 / im.h)).max(1);
    let dw = (im.w * scale / 1024).clamp(1, cw);
    let dh = (im.h * scale / 1024).clamp(1, ch);
    let ox = (cw - dw) / 2;
    let oy = (ch - dh) / 2;
    for dy in 0..dh {
        let sy = dy * im.h / dh;
        let srow = sy * im.w;
        for dx in 0..dw {
            let sx = dx * im.w / dw;
            fb.put_pixel(ox + dx, oy + dy, im.pixels[srow + sx]);
        }
    }
}
