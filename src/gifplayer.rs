//! M31: the animated GIF player app. Lists every `.GIF` on the FAT16 disk,
//! decodes it with our from-scratch `gif` module, and plays the
//! pre-composited frames on the desktop timer. Space toggles play/pause,
//! left/right scrub frames, up/down switch files, Escape closes. Frames blit
//! centred + nearest-neighbour scaled, same as the image viewer.

use crate::wm::Window;
use crate::{fs, gif, kprintln, timer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0xff0a_0c10;
const TICK_HZ: u64 = 50; // desktop timer rate (matches clock::HZ)

/// What a keypress did, for the WM to act on (close lives at the WM level).
pub enum Action {
    None,
    Redraw,
    Close,
}

pub struct GifPlayerState {
    files: Vec<String>, // all .GIF names on disk, sorted
    idx: usize,         // which file is loaded
    gif: Option<gif::Gif>,
    frame: usize,
    next_tick: u64, // timer::ticks() when to advance the frame
    playing: bool,
}

impl GifPlayerState {
    pub fn new() -> GifPlayerState {
        let mut files: Vec<String> = fs::list_root()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|n| n.ends_with(".GIF"))
            .collect();
        files.sort();
        let mut st = GifPlayerState {
            files,
            idx: 0,
            gif: None,
            frame: 0,
            next_tick: 0,
            playing: true,
        };
        st.load();
        st
    }

    /// Like ViewerState::with_file — open a specific GIF (Files "open").
    pub fn with_file(name: &str) -> GifPlayerState {
        let mut st = GifPlayerState::new();
        if let Some(i) = st.files.iter().position(|f| f == name) {
            st.idx = i;
            st.load();
        }
        st
    }

    fn load(&mut self) {
        self.gif = None;
        self.frame = 0;
        let Some(name) = self.files.get(self.idx) else {
            kprintln!("GIF: no .GIF files on disk");
            return;
        };
        match fs::read_file(name).and_then(|d| gif::decode(&d)) {
            Some(g) => {
                kprintln!("GIF: {name} {}x{}, {} frames", g.w, g.h, g.frames.len());
                self.gif = Some(g);
                self.next_tick = timer::ticks() + self.cur_delay();
                kprintln!("GIF_OK");
            }
            None => kprintln!("GIF: cannot decode {name}"),
        }
    }

    fn nframes(&self) -> usize {
        self.gif.as_ref().map_or(0, |g| g.frames.len())
    }

    /// Ticks to hold the current frame. delay_cs is centiseconds; the timer
    /// runs at 50 Hz, so ticks = cs * 50 / 100. 0 -> a 10cs (100 ms) default.
    fn cur_delay(&self) -> u64 {
        let cs = self
            .gif
            .as_ref()
            .and_then(|g| g.frames.get(self.frame))
            .map_or(0, |f| f.delay_cs as u64);
        let cs = if cs == 0 { 10 } else { cs };
        (cs * TICK_HZ / 100).max(1)
    }

    pub fn title(&self) -> String {
        let name = self.files.get(self.idx).cloned().unwrap_or_else(|| String::from("no gif"));
        let n = self.nframes();
        if n == 0 {
            return name;
        }
        let state = if self.playing { "PLAYING" } else { "PAUSED" };
        format!("{} [{}/{}] {}", name, self.frame + 1, n, state)
    }

    fn step_frame(&mut self, d: isize) {
        let n = self.nframes();
        if n == 0 {
            return;
        }
        self.frame = (((self.frame as isize + d) % n as isize + n as isize) % n as isize) as usize;
        self.next_tick = timer::ticks() + self.cur_delay();
    }

    fn step_file(&mut self, d: isize) {
        let n = self.files.len();
        if n == 0 {
            return;
        }
        self.idx = (((self.idx as isize + d) % n as isize + n as isize) % n as isize) as usize;
        self.load();
    }
}

/// Keys: space play/pause, left/right frame scrub, up/down switch files,
/// Escape close.
pub fn key(win: &mut Window, code: u16) -> Action {
    const ESC: u16 = 1;
    const SPACE: u16 = 57;
    const LEFT: u16 = 105;
    const RIGHT: u16 = 106;
    const UP: u16 = 103;
    const DOWN: u16 = 108;
    {
        let crate::wm::App::Gif(st) = &mut win.app else { return Action::None };
        match code {
            ESC => return Action::Close,
            SPACE => {
                st.playing = !st.playing;
                if st.playing {
                    st.next_tick = timer::ticks() + st.cur_delay();
                }
            }
            LEFT => {
                st.playing = false;
                st.step_frame(-1);
            }
            RIGHT => {
                st.playing = false;
                st.step_frame(1);
            }
            UP => st.step_file(-1),
            DOWN => st.step_file(1),
            _ => return Action::None,
        }
        win.title = st.title();
    }
    render(win);
    Action::Redraw
}

/// Click anywhere toggles play/pause (a friendly default).
pub fn click(win: &mut Window) {
    {
        let crate::wm::App::Gif(st) = &mut win.app else { return };
        st.playing = !st.playing;
        if st.playing {
            st.next_tick = timer::ticks() + st.cur_delay();
        }
        win.title = st.title();
    }
    render(win);
}

/// Auto-advance the animation; returns true if it repainted.
pub fn tick(win: &mut Window, now: u64) -> bool {
    {
        let crate::wm::App::Gif(st) = &mut win.app else { return false };
        if !st.playing || st.nframes() == 0 || now < st.next_tick {
            return false;
        }
        st.frame = (st.frame + 1) % st.nframes();
        st.next_tick = now + st.cur_delay();
        win.title = st.title();
    }
    render(win);
    true
}

pub fn render(win: &mut Window) {
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    let crate::wm::App::Gif(st) = &win.app else { return };
    let Some(g) = &st.gif else {
        let msg = if st.files.is_empty() { "no .GIF files on disk" } else { "cannot decode GIF" };
        fb.draw_string(8, 8, msg, 0xffd0_d8e0, None);
        return;
    };
    let Some(frame) = g.frames.get(st.frame) else { return };
    let (iw, ih) = (g.w as usize, g.h as usize);
    if iw == 0 || ih == 0 {
        return;
    }
    let scale = ((cw * 1024 / iw).min(ch * 1024 / ih)).max(1);
    let dw = (iw * scale / 1024).clamp(1, cw);
    let dh = (ih * scale / 1024).clamp(1, ch);
    let ox = (cw - dw) / 2;
    let oy = (ch - dh) / 2;
    for dy in 0..dh {
        let sy = dy * ih / dh;
        let srow = sy * iw;
        for dx in 0..dw {
            let sx = dx * iw / dw;
            fb.put_pixel(ox + dx, oy + dy, frame.pixels[srow + sx]);
        }
    }
}
