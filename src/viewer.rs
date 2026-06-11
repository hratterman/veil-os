//! M23: the image viewer app. Opens every `.PNG` on the FAT16 disk
//! (alphabetical), decoding with the kernel's own `png` module, and shows
//! one at a time scaled to fit the window with nearest-neighbour sampling.
//! Left/right arrows cycle through the images; the window title is the
//! current filename. Images smaller than the window are centred on a flat
//! background; larger ones are shrunk to fit, aspect preserved.

use crate::fb::Framebuffer;
use crate::wm::Window;
use crate::{fs, kprintln, png};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff14_181c; // letterbox / background fill behind the image

/// Why the current file can't be shown — drives the on-screen message so the
/// user sees the filename, real dimensions and size instead of a black box.
struct BadImg {
    w: usize, // real PNG dimensions, 0 if the header couldn't be parsed
    h: usize,
    bytes: usize,    // file size on disk
    too_large: bool, // exceeded the decoder's max dimensions
}

pub struct ViewerState {
    files: Vec<String>,      // sorted .PNG names on the disk
    idx: usize,              // current image
    img: Option<png::Image>, // decoded current image (None = undecodable)
    bad: Option<BadImg>,     // set when the current file failed to decode
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
        let mut st = ViewerState { files, idx: 0, img: None, bad: None };
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

    /// Decode the current file and log it (the proof greps these lines). On
    /// failure, probe the header for the real dimensions so the viewer can show
    /// a useful message rather than a black "cannot decode" box.
    fn load(&mut self) {
        self.img = None;
        self.bad = None;
        let Some(name) = self.files.get(self.idx).cloned() else {
            kprintln!("VIEWER: no .PNG files on disk");
            return;
        };
        let Some(data) = fs::read_file(&name) else {
            kprintln!("VIEWER: cannot read {name}");
            self.bad = Some(BadImg { w: 0, h: 0, bytes: 0, too_large: false });
            return;
        };
        let bytes = data.len();
        match png::decode(&data) {
            Some(im) => {
                if im.w == im.full_w && im.h == im.full_h {
                    kprintln!("VIEWER: showing {name} {}x{}", im.full_w, im.full_h);
                } else {
                    kprintln!(
                        "VIEWER: showing {name} {}x{} (downscaled to {}x{})",
                        im.full_w, im.full_h, im.w, im.h
                    );
                }
                self.img = Some(im);
            }
            None => {
                let (w, h) = png::probe(&data).unwrap_or((0, 0));
                let too_large = w > 2048 || h > 2048;
                kprintln!("VIEWER: cannot decode {name} ({w}x{h}, {bytes} bytes)");
                self.bad = Some(BadImg { w, h, bytes, too_large });
            }
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
        render_error(&fb, st);
        return;
    };
    if im.w == 0 || im.h == 0 {
        return;
    }
    // Fit to the content box, preserving aspect (scale in 1/1024 units). The
    // destination is always shrunk to the window, so even a 2048px image only
    // ever blits `cw`x`ch` pixels — large images are scaled down, never drawn
    // 1:1 past the canvas edge.
    let scale = ((cw * 1024 / im.w).min(ch * 1024 / im.h)).max(1);
    let dw = (im.w * scale / 1024).clamp(1, cw);
    let dh = (im.h * scale / 1024).clamp(1, ch);
    let ox = (cw - dw) / 2;
    let oy = (ch - dh) / 2;
    for dy in 0..dh {
        let sy = (dy * im.h / dh).min(im.h - 1);
        let srow = sy * im.w;
        for dx in 0..dw {
            let sx = (dx * im.w / dw).min(im.w - 1);
            let si = srow + sx;
            // Guard both ends of the blit: the source index into the decoded
            // buffer and the destination pixel inside the canvas.
            if si < im.pixels.len() && ox + dx < cw && oy + dy < ch {
                fb.put_pixel(ox + dx, oy + dy, im.pixels[si]);
            }
        }
    }
}

/// Friendly multi-line message for a file that couldn't be shown: filename,
/// real dimensions, size, and the reason — in the viewer's retro style.
fn render_error(fb: &Framebuffer, st: &ViewerState) {
    const NAME_COL: u32 = 0xffd0_d8e0;
    const DIM_COL: u32 = 0xff8a_94a0;
    const WARN_COL: u32 = 0xffe0_b070;

    if st.files.is_empty() {
        fb.draw_string(12, 24, "No .PNG files on disk", NAME_COL, None);
        return;
    }

    let mut y = 24usize;
    fb.draw_string(12, y, &st.current_name(), NAME_COL, None);
    y += 26;
    if let Some(b) = &st.bad {
        if b.w > 0 {
            fb.draw_string(12, y, &format!("{} x {} px", b.w, b.h), DIM_COL, None);
            y += 18;
        }
        if b.bytes > 0 {
            fb.draw_string(12, y, &human_size(b.bytes), DIM_COL, None);
            y += 18;
        }
        y += 8;
        let msg = if b.too_large {
            "Image too large for Veil - max 2048 x 2048 px"
        } else {
            "Could not decode this image"
        };
        fb.draw_string(12, y, msg, WARN_COL, None);
    } else {
        fb.draw_string(12, y, "Could not decode this image", WARN_COL, None);
    }
}

/// Render a byte count as a compact human string (e.g. "3.8 MB", "92.3 KB").
fn human_size(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        let tenths = (bytes * 10) >> 20;
        format!("{}.{} MB", tenths / 10, tenths % 10)
    } else if bytes >= 1 << 10 {
        let tenths = (bytes * 10) >> 10;
        format!("{}.{} KB", tenths / 10, tenths % 10)
    } else {
        format!("{bytes} bytes")
    }
}
