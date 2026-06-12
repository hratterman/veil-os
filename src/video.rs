//! M35 MJPEG video player. An MJPEG file is a sequence of baseline JPEG frames;
//! with the JPEG decoder this is a frame index + a decode-on-tick loop. Handles
//! a raw concatenated-JPEG stream and a basic Motion-JPEG AVI (RIFF) container.
//! Space toggles play/pause; left/right seek. Frames scale to fit the window.

use crate::wm::{App, Window};
use crate::png::Image;
use crate::{fs, jpeg, kprintln, timer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff0d_0d0d;
const TEXT: u32 = 0xffe8_e8e8;
const STEP: u64 = 2; // ticks per frame (~25 fps at the 50 Hz timer)

pub struct VideoState {
    name: String,
    data: Vec<u8>,
    frames: Vec<(usize, usize)>, // byte ranges of each JPEG frame (MJPEG)
    predecoded: Vec<Image>,      // pre-decoded frames (H.264 .mp4)
    cur: usize,
    playing: bool,
    next_tick: u64,
    img: Option<Image>,
    decoded_frame: usize,
}

/// Find every JPEG frame (SOI..next SOI) in a raw or AVI-wrapped MJPEG stream.
fn split_frames(data: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 1 < data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xD8 && (i + 2 >= data.len() || data[i + 2] == 0xFF) {
            starts.push(i);
            i += 2;
        } else {
            i += 1;
        }
    }
    let mut frames = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(data.len());
        frames.push((s, e));
    }
    frames
}

impl VideoState {
    pub fn with_file(name: &str) -> VideoState {
        let data = fs::read_file(name).unwrap_or_default();
        Self::with_data(String::from(name), data)
    }

    /// Build a player from in-memory bytes (e.g. an MP4 the browser fetched for
    /// a `<video>` tag). `.mp4`/`.MP4` is decoded as H.264, else treated as MJPEG.
    pub fn with_data(name: String, data: Vec<u8>) -> VideoState {
        let is_mp4 = name.ends_with(".MP4") || name.ends_with(".mp4");
        let mut predecoded = Vec::new();
        let frames;
        if is_mp4 {
            let fr = crate::h264::decode_all(&data, 120);
            predecoded = fr
                .into_iter()
                .map(|f| Image { w: f.w, h: f.h, full_w: f.w, full_h: f.h, pixels: f.pixels })
                .collect::<Vec<_>>();
            // Dummy byte ranges so the play loop's `cur % frames.len()` works.
            frames = (0..predecoded.len()).map(|i| (i, i)).collect::<Vec<_>>();
            kprintln!("VIDEO: {name} -> H.264, {} frames ({} bytes)", predecoded.len(), data.len());
        } else {
            frames = split_frames(&data);
            kprintln!("VIDEO: {name} -> {} frames ({} bytes)", frames.len(), data.len());
        }
        let mut st = VideoState {
            name,
            data,
            frames,
            predecoded,
            cur: 0,
            playing: true,
            next_tick: 0,
            img: None,
            decoded_frame: usize::MAX,
        };
        st.decode_current();
        st
    }

    fn decode_current(&mut self) {
        if self.decoded_frame == self.cur {
            return;
        }
        if !self.predecoded.is_empty() {
            self.img = self.predecoded.get(self.cur).cloned();
            self.decoded_frame = self.cur;
            if self.cur == 0 {
                if let Some(im) = &self.img {
                    kprintln!("VIDEO: frame 0 {}x{}", im.w, im.h);
                }
            }
            return;
        }
        if let Some(&(s, e)) = self.frames.get(self.cur) {
            self.img = jpeg::decode(&self.data[s..e]);
            self.decoded_frame = self.cur;
            if self.cur == 0 || self.img.is_none() {
                match &self.img {
                    Some(im) => kprintln!("VIDEO: frame {} {}x{}", self.cur, im.w, im.h),
                    None => kprintln!("VIDEO: frame {} failed to decode", self.cur),
                }
            }
        }
    }

    pub fn title(&self) -> String {
        let state = if self.playing { "play" } else { "pause" };
        format!("{} [{}/{} {state}]", self.name, self.cur + 1, self.frames.len().max(1))
    }
}

pub fn tick(win: &mut Window, now: u64) -> bool {
    {
        let App::Video(st) = &mut win.app else { return false };
        if !st.playing || st.frames.is_empty() || now < st.next_tick {
            return false;
        }
        st.next_tick = now + STEP;
        st.cur = (st.cur + 1) % st.frames.len();
        st.decode_current();
        win.title = st.title();
    }
    render(win);
    true
}

pub fn key(win: &mut Window, code: u16) -> bool {
    const SPACE: u16 = 57;
    const LEFT: u16 = 105;
    const RIGHT: u16 = 106;
    {
        let App::Video(st) = &mut win.app else { return false };
        if st.frames.is_empty() {
            return false;
        }
        match code {
            SPACE => st.playing = !st.playing,
            LEFT => {
                st.cur = (st.cur + st.frames.len() - 1) % st.frames.len();
                st.decode_current();
            }
            RIGHT => {
                st.cur = (st.cur + 1) % st.frames.len();
                st.decode_current();
            }
            _ => return false,
        }
        win.title = st.title();
    }
    render(win);
    true
}

pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    let App::Video(st) = &win.app else { return };
    let Some(im) = &st.img else {
        fb.draw_string(8, 8, "video: cannot decode frame", TEXT, None);
        return;
    };
    if im.w == 0 || im.h == 0 {
        return;
    }
    // Aspect-fit, nearest-neighbour (same as the image viewer).
    let scale = ((cw * 1024 / im.w).min(ch * 1024 / im.h)).max(1);
    let dw = (im.w * scale / 1024).clamp(1, cw);
    let dh = (im.h * scale / 1024).clamp(1, ch);
    let (ox, oy) = ((cw - dw) / 2, (ch - dh) / 2);
    for dy in 0..dh {
        let sy = (dy * im.h / dh).min(im.h - 1);
        let srow = sy * im.w;
        for dx in 0..dw {
            let sx = (dx * im.w / dw).min(im.w - 1);
            let si = srow + sx;
            if si < im.pixels.len() && ox + dx < cw && oy + dy < ch {
                fb.put_pixel(ox + dx, oy + dy, im.pixels[si]);
            }
        }
    }
}
