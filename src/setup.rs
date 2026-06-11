//! M27: first-boot setup screen. Shown once, before the desktop, when the
//! FAT16 disk has no `USER.TXT`. The visitor types a username and picks a
//! UTC offset (arrow keys, 30-min steps, UTC-12..UTC+14); on Enter we write
//! `USER.TXT` + `TZ.TXT`, apply the timezone to the (already NTP-anchored)
//! wall clock, and fall through to the normal desktop. Subsequent boots see
//! a non-empty USER.TXT and skip straight to the desktop.

use crate::fb::Framebuffer;
use crate::{fs, input, keymap, kprintln, timer};
use alloc::string::String;

const BG: u32 = 0xff0d_0d0d;
const CARD: u32 = 0xff1a_1a1a;
const CARD_EDGE: u32 = 0xff2a_2a2a;
const HEAD: u32 = 0xffe8_e8e8;
const LABEL: u32 = 0xff88_8888;
const FIELD_BG: u32 = 0xff14_1414;
const FIELD_TX: u32 = 0xffe8_e8e8;
const ACCENT: u32 = 0xff5b_8af0;
const BTN_BG: u32 = 0xff5b_8af0;
const BTN_TX: u32 = 0xff0d_0d0d;

const MIN_HALF: i32 = -24; // UTC-12:00
const MAX_HALF: i32 = 28; //  UTC+14:00
const NAME_MAX: usize = 20;

const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;

/// Does this boot need the setup screen? True when a filesystem is mounted
/// but USER.TXT is absent or blank. With no disk (the diskless GUI proofs)
/// there's nowhere to persist a name, so go straight to the desktop.
pub fn needed() -> bool {
    if !fs::mounted() {
        return false;
    }
    match fs::read_file("USER.TXT") {
        Some(d) => core::str::from_utf8(&d).map(|s| s.trim().is_empty()).unwrap_or(true),
        None => true,
    }
}

/// Run the full-screen setup loop. Returns once the user confirms a
/// non-empty name (files written, timezone applied). Assumes input is up
/// and a timer tick is running (for the blinking cursor).
pub fn run(screen: &Framebuffer) {
    kprintln!("SETUP: first boot, no USER.TXT — showing setup screen");
    let mut name = String::new();
    let mut half: i32 = 0; // UTC offset in 30-min steps; 0 = UTC+0
    let mut shift = false;
    let mut blink = true;
    render_static(screen);
    render(screen, &name, half, blink);
    let mut last_blink = timer::ticks();

    loop {
        while let Some((ev, code, value)) = input::pop() {
            if ev != keymap::EV_KEY {
                continue;
            }
            if matches!(code, keymap::KEY_LEFTSHIFT | keymap::KEY_RIGHTSHIFT) {
                shift = value != 0;
                continue;
            }
            if value != 1 {
                continue; // key-down only
            }
            match code {
                KEY_LEFT => {
                    half = (half - 1).max(MIN_HALF);
                    blink = true;
                    render(screen, &name, half, blink);
                }
                KEY_RIGHT => {
                    half = (half + 1).min(MAX_HALF);
                    blink = true;
                    render(screen, &name, half, blink);
                }
                _ => {
                    let Some(ch) = keymap::translate(code, shift) else { continue };
                    match ch {
                        '\n' => {
                            if !name.trim().is_empty() {
                                commit(name.trim(), half);
                                return;
                            }
                        }
                        '\u{8}' => {
                            name.pop();
                        }
                        c if (' '..='~').contains(&c) => {
                            if name.chars().count() < NAME_MAX {
                                name.push(c);
                            }
                        }
                        _ => {}
                    }
                    blink = true;
                    render(screen, &name, half, blink);
                }
            }
        }
        // ~2 Hz cursor blink (50 Hz tick).
        let now = timer::ticks();
        if now.saturating_sub(last_blink) >= 25 {
            blink = !blink;
            last_blink = now;
            render(screen, &name, half, blink);
        }
        unsafe { core::arch::asm!("wfi") };
    }
}

/// "UTC+5:30" / "UTC-12:00" for display.
fn tz_label(half: i32) -> String {
    let neg = half < 0;
    let a = half.unsigned_abs();
    alloc::format!("UTC{}{}:{:02}", if neg { '-' } else { '+' }, a / 2, (a % 2) * 30)
}

/// The value written to TZ.TXT: hours, with ".5" for half-hour zones.
fn tz_value(half: i32) -> String {
    let neg = half < 0;
    let a = half.unsigned_abs();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    let _ = core::fmt::write(&mut s, format_args!("{}", a / 2));
    if a % 2 == 1 {
        s.push_str(".5");
    }
    s
}

fn commit(name: &str, half: i32) {
    let user = alloc::format!("{name}\n");
    if fs::write_file("USER.TXT", user.as_bytes()).is_err() {
        kprintln!("SETUP: WARNING — USER.TXT write failed");
    }
    let tz = tz_value(half);
    let tz_file = alloc::format!("{tz}\n");
    if fs::write_file("TZ.TXT", tz_file.as_bytes()).is_err() {
        kprintln!("SETUP: WARNING — TZ.TXT write failed");
    }
    // Apply the offset immediately so the clock shows local time (the wall
    // clock was already NTP-anchored at boot).
    timer::set_tz(half as i64 * 1800);
    kprintln!("SETUP: name='{name}' tz={tz} ({})", tz_label(half));
    kprintln!("SETUP_OK");
}

/// Draw the static parts (background, card, labels) once.
fn render_static(screen: &Framebuffer) {
    let (w, h) = (screen.width, screen.height);
    screen.clear(BG);
    let cw = 460usize;
    let ch = 320usize;
    let cx = (w - cw) / 2;
    let cy = (h - ch) / 2;
    use crate::freetype::FontId::{Ui, UiBold};
    screen.fill_round_rect(cx - 1, cy - 1, cw + 2, ch + 2, 12, CARD_EDGE);
    screen.fill_round_rect(cx, cy, cw, ch, 12, CARD);
    screen.draw_text(cx + 28, cy + 22, "Welcome to Veil OS", UiBold, 22, HEAD);
    screen.draw_text(cx + 30, cy + 62, "a bare-metal AArch64 operating system", Ui, 15, LABEL);
    let fy = cy + 110;
    screen.draw_text(cx + 30, fy, "Your name", Ui, 15, LABEL);
    let ty = cy + 180;
    screen.draw_text(cx + 30, ty, "Timezone  (left / right arrows)", Ui, 15, LABEL);
    screen.draw_text(cx + 36, ty + 22, "<", Ui, 18, ACCENT);
    screen.draw_text(cx + cw - 50, ty + 22, ">", Ui, 18, ACCENT);
}

/// Redraw only the dynamic fields (name input, timezone, button). No full clear.
fn render(screen: &Framebuffer, name: &str, half: i32, blink: bool) {
    let (w, h) = (screen.width, screen.height);
    let cw = 460usize;
    let ch = 320usize;
    let cx = (w - cw) / 2;
    let cy = (h - ch) / 2;

    use crate::freetype::FontId::Ui;
    // Name field: rounded input with an accent border (focus ring).
    let fy = cy + 110;
    screen.fill_round_rect(cx + 30, fy + 20, cw - 60, 32, 6, ACCENT);
    screen.fill_round_rect(cx + 31, fy + 21, cw - 62, 30, 5, FIELD_BG);
    let cursor = if blink { "|" } else { " " };
    screen.draw_text(cx + 38, fy + 27, &alloc::format!("{name}{cursor}"), Ui, 16, FIELD_TX);

    // Timezone field.
    let ty = cy + 180;
    screen.fill_round_rect(cx + 30, ty + 20, cw - 60, 32, 6, FIELD_BG);
    screen.draw_text(cx + 36, ty + 22, "<", Ui, 18, ACCENT);
    screen.draw_text(cx + cw - 50, ty + 22, ">", Ui, 18, ACCENT);
    let lbl = tz_label(half);
    let (lw, _) = screen.measure_text(&lbl, Ui, 18);
    screen.draw_text(cx + cw / 2 - lw / 2, ty + 26, &lbl, Ui, 18, FIELD_TX);

    // Button: accent when a name is entered.
    let by = cy + ch - 56;
    let valid = !name.trim().is_empty();
    let bg = if valid { BTN_BG } else { 0xff2a_2a2a };
    screen.fill_round_rect(cx + 30, by, cw - 60, 36, 8, bg);
    let hint = if valid { "Press Enter to continue" } else { "Type a name to continue" };
    let (hw, _) = screen.measure_text(hint, Ui, 15);
    let htx = if valid { BTN_TX } else { 0xff888888 };
    screen.draw_text(cx + cw / 2 - hw / 2, by + 9, hint, Ui, 15, htx);
}
