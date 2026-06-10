//! M19: the clock app. Four faces cycled by clicking the face — wall
//! (analog, Arabic numerals), digital (HH:MM:SS), chronograph (analog
//! minutes dial + seconds/hours sub-dials), stopwatch (MM:SS.cc) — all
//! driven from the 50 Hz desktop timer tick, redrawn every 100 ms so the
//! analog second hand sweeps instead of stepping.
//!
//! No RTC hardware in scope: wall time is time-since-boot. The
//! chronograph and stopwatch faces share one start/stop accumulator
//! (spec), controlled by the STA/STP/RST buttons at the bottom of those
//! two faces. Clicks on the buttons do not cycle the face.
//!
//! Trig without floats or tables: Bhaskara I's sine approximation in
//! 0.1-degree fixed point, good to ~0.2% — far below a pixel at r<120.

use crate::fb::Framebuffer;
use crate::wm::Window;
use crate::{kprintln, timer};
use alloc::format;
use alloc::string::String;

pub const HZ: u64 = 50; // desktop timer rate (desktop.rs starts 50 Hz)
const SLOT_TICKS: u64 = HZ / 10; // redraw every 100 ms

const BTN_H: isize = 34; // chrono/stopwatch button strip height
const FACE_NAMES: [&str; 4] = ["wall", "digital", "chrono", "stopwatch"];

const LIGHT_BG: u32 = 0xfff4_f2ec;
const DARK_BG: u32 = 0xff10_1418;
const RIM: u32 = 0xff30_3840;
const HOUR_HAND: u32 = 0xff20_2830;
const MIN_HAND: u32 = 0xff40_5060;
const SEC_HAND: u32 = 0xffd0_3030;
const DIGITS: u32 = 0xff60_e0a0;

pub struct ClockState {
    face: usize,
    epoch: u64,     // tick at creation = boot zero for the wall faces
    running: bool,  // chrono/stopwatch engine (shared accumulator)
    accum: u64,     // ticks banked while stopped
    started: u64,   // tick of the last STA press (valid while running)
    last_slot: u64, // last 100 ms slot drawn
}

impl ClockState {
    pub fn new() -> ClockState {
        ClockState {
            face: 0,
            epoch: timer::ticks(),
            running: false,
            accum: 0,
            started: 0,
            last_slot: u64::MAX,
        }
    }

    fn sw_ticks(&self, now: u64) -> u64 {
        self.accum + if self.running { now - self.started } else { 0 }
    }
}

/// Bhaskara I: sin(a) scaled by 1024, `a` in 0.1-degree units.
fn sin1024(a: i64) -> i64 {
    let a = a.rem_euclid(3600);
    let (a, sign) = if a >= 1800 { (a - 1800, -1) } else { (a, 1) };
    let p = a * (1800 - a);
    sign * 4 * p * 1024 / (4_050_000 - p)
}

/// Unit vector (x1024) for a hand at `frac`/`of` of a clockwise
/// revolution from 12 o'clock: (sin, -cos).
fn hand_vec(frac: u64, of: u64) -> (i64, i64) {
    let a = (frac % of) as i64 * 3600 / of as i64;
    (sin1024(a), -sin1024(a + 900))
}

fn draw_hand(fb: &Framebuffer, cx: isize, cy: isize, frac: u64, of: u64, len: isize, color: u32, thick: bool) {
    let (dx, dy) = hand_vec(frac, of);
    let (ex, ey) = (cx + (dx * len as i64 / 1024) as isize, cy + (dy * len as i64 / 1024) as isize);
    fb.draw_line(cx, cy, ex, ey, color);
    if thick {
        // 3px: a plus-shaped offset bundle around the Bresenham core.
        for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            fb.draw_line(cx + ox, cy + oy, ex + ox, ey + oy, color);
        }
    }
}

/// Dial: rim circle, tick marks, optional numerals, three hands reading
/// (hour-frac/of, min-frac/of, sec-frac/of). Used by wall + chrono dials.
fn draw_ticks(fb: &Framebuffer, cx: isize, cy: isize, r: isize, every5: bool) {
    for i in 0..60u64 {
        if every5 && i % 5 != 0 {
            continue;
        }
        let (dx, dy) = hand_vec(i, 60);
        let inner = if i % 5 == 0 { r - 8 } else { r - 4 };
        let (x0, y0) = (cx + (dx * inner as i64 / 1024) as isize, cy + (dy * inner as i64 / 1024) as isize);
        let (x1, y1) = (cx + (dx * r as i64 / 1024) as isize, cy + (dy * r as i64 / 1024) as isize);
        fb.draw_line(x0, y0, x1, y1, RIM);
    }
}

/// Timezone label for synced time ("UTC", "UTC-5", "UTC+1"), or the
/// "(no sync)" indicator when NTP never anchored real time.
fn time_label() -> String {
    if !timer::synced() {
        return String::from("(no sync)");
    }
    let h = timer::tz_offset_seconds() / 3600;
    if h == 0 { String::from("UTC") } else { format!("UTC{h:+}") }
}

fn render_wall(fb: &Framebuffer, cw: usize, ch: usize, t: u64) {
    fb.clear(LIGHT_BG);
    let (cx, cy) = (cw as isize / 2, ch as isize / 2);
    let r = (cx.min(cy)) - 8;
    fb.draw_circle(cx, cy, r, RIM);
    fb.draw_circle(cx, cy, r - 1, RIM);
    draw_ticks(fb, cx, cy, r - 2, false);
    for n in 1..=12u64 {
        let (dx, dy) = hand_vec(n, 12);
        let nr = r - 22;
        let s = format!("{n}");
        let x = cx + (dx * nr as i64 / 1024) as isize - s.len() as isize * 4;
        let y = cy + (dy * nr as i64 / 1024) as isize - 8;
        fb.draw_string(x.max(0) as usize, y.max(0) as usize, &s, HOUR_HAND, None);
    }
    // Timezone / sync label inside the dial, between centre and the "6" —
    // clear of the rim (the old "since boot" text overlapped the face).
    let label = time_label();
    let lx = cx as usize - label.len() * 4;
    fb.draw_string(lx, (cy + r / 2) as usize - 8, &label, MIN_HAND, None);
    draw_hand(fb, cx, cy, t, HZ * 43200, r * 5 / 10, HOUR_HAND, true);
    draw_hand(fb, cx, cy, t, HZ * 3600, r * 7 / 10, MIN_HAND, true);
    draw_hand(fb, cx, cy, t, HZ * 60, r * 9 / 10, SEC_HAND, false);
    fb.fill_rect(cx as usize - 2, cy as usize - 2, 5, 5, SEC_HAND);
}

fn render_digital(fb: &Framebuffer, cw: usize, ch: usize, t: u64, synced: bool) {
    fb.clear(DARK_BG);
    let s = t / HZ;
    // Real local time wraps at 24h; time-since-boot counts hours up.
    let hh = if synced { (s / 3600) % 24 } else { s / 3600 };
    let text = format!("{:02}:{:02}:{:02}", hh, (s / 60) % 60, s % 60);
    fb.draw_string_scaled((cw - text.len() * 24) / 2, ch / 2 - 24, &text, DIGITS, 3);
    let label = time_label();
    fb.draw_string((cw - label.len() * 8) / 2, ch / 2 + 32, &label, 0xff60_7080, None);
}

fn render_buttons(fb: &Framebuffer, cw: usize, ch: usize, running: bool) {
    let y = ch - BTN_H as usize + 4;
    let bw = (cw - 16) / 3;
    let labels: [(&str, u32, u32); 3] = [
        ("STA", 0xffb0_e0b8, 0xff20_6030),
        ("STP", 0xffe0_c0a0, 0xff80_4010),
        ("RST", 0xffe0_b0b0, 0xff80_2020),
    ];
    for (i, (label, bg, fg)) in labels.iter().enumerate() {
        let x = 4 + i * (bw + 4);
        let bg = if (i == 0 && running) || (i == 1 && !running) { 0xffd8_dce0 } else { *bg };
        fb.fill_rect(x, y, bw, BTN_H as usize - 8, bg);
        fb.draw_string(x + bw / 2 - 12, y + 6, label, *fg, None);
    }
}

fn render_chrono(fb: &Framebuffer, cw: usize, ch: usize, sw: u64, running: bool) {
    fb.clear(LIGHT_BG);
    let (cx, cy) = (cw as isize / 2, (ch as isize - BTN_H) / 2);
    let r = cx.min(cy) - 6;
    fb.draw_circle(cx, cy, r, RIM);
    draw_ticks(fb, cx, cy, r - 2, false);
    // Main dial: minutes elapsed, one rev per hour.
    draw_hand(fb, cx, cy, sw, HZ * 3600, r * 8 / 10, HOUR_HAND, true);
    // Sub-dial left: seconds (rev/min, smooth sweep).
    let (sx, sy, sr) = (cx - r / 2, cy, r * 28 / 100);
    fb.draw_circle(sx, sy, sr, RIM);
    draw_ticks(fb, sx, sy, sr, true);
    draw_hand(fb, sx, sy, sw, HZ * 60, sr - 3, SEC_HAND, false);
    // Sub-dial right: hours (rev/12h).
    let (hx, hy, hr) = (cx + r / 2, cy, r * 28 / 100);
    fb.draw_circle(hx, hy, hr, RIM);
    draw_ticks(fb, hx, hy, hr, true);
    draw_hand(fb, hx, hy, sw, HZ * 43200, hr - 3, MIN_HAND, true);
    fb.fill_rect(cx as usize - 1, cy as usize - 1, 3, 3, HOUR_HAND);
    render_buttons(fb, cw, ch, running);
}

fn render_stopwatch(fb: &Framebuffer, cw: usize, ch: usize, sw: u64, running: bool) {
    fb.clear(DARK_BG);
    let cs = sw * 100 / HZ; // centiseconds
    let text = format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100);
    fb.draw_string_scaled((cw - text.len() * 24) / 2, (ch - BTN_H as usize) / 2 - 24, &text, DIGITS, 3);
    render_buttons(fb, cw, ch, running);
}

pub fn render(win: &mut Window, now: u64) {
    let (cw, ch) = (win.cw, win.ch);
    let (face, boot_t, sw, running) = {
        let crate::wm::App::Clock(st) = &win.app else { return };
        (st.face, now - st.epoch, st.sw_ticks(now), st.running)
    };
    // Wall + digital faces show real local time once NTP has synced;
    // otherwise they fall back to time-since-boot (chrono/stopwatch are
    // always elapsed timers, unaffected).
    let synced = timer::synced();
    let wall_t = timer::wall_ticks50().unwrap_or(boot_t);
    let fb = win.canvas_fb();
    match face {
        0 => render_wall(&fb, cw, ch, wall_t),
        1 => render_digital(&fb, cw, ch, wall_t, synced),
        2 => render_chrono(&fb, cw, ch, sw, running),
        _ => render_stopwatch(&fb, cw, ch, sw, running),
    }
}

/// 100 ms cadence redraw, called from the desktop loop via Wm::clock_tick.
/// Returns true if the window was repainted.
pub fn tick(win: &mut Window, now: u64) -> bool {
    let slot = now / SLOT_TICKS;
    {
        let crate::wm::App::Clock(st) = &mut win.app else { return false };
        if slot == st.last_slot {
            return false;
        }
        st.last_slot = slot;
    }
    render(win, now);
    true
}

/// Canvas-relative click: STA/STP/RST on the two timer faces, anywhere
/// else cycles to the next face.
pub fn click(win: &mut Window, rx: isize, ry: isize) {
    let now = timer::ticks();
    let (cw, ch) = (win.cw as isize, win.ch as isize);
    {
        let crate::wm::App::Clock(st) = &mut win.app else { return };
        let on_buttons = st.face >= 2 && ry >= ch - BTN_H + 4;
        if on_buttons {
            let bw = (cw - 16) / 3;
            let btn = ((rx - 4) / (bw + 4)).clamp(0, 2);
            match btn {
                0 if !st.running => {
                    st.running = true;
                    st.started = now;
                    kprintln!("CLOCK: start (accum {} ticks)", st.accum);
                }
                1 if st.running => {
                    st.accum += now - st.started;
                    st.running = false;
                    kprintln!("CLOCK: stop at {} ticks", st.accum);
                }
                2 => {
                    st.accum = 0;
                    if st.running {
                        st.started = now;
                    }
                    kprintln!("CLOCK: reset");
                }
                _ => {}
            }
        } else {
            st.face = (st.face + 1) % 4;
            kprintln!("CLOCK: face -> {}", FACE_NAMES[st.face]);
        }
    }
    render(win, now);
}

