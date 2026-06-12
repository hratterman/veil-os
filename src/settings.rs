//! M36 settings app: Display / Sound / System / About pages, persisted to
//! SETTINGS.TXT (key=value).

use crate::freetype::FontId;
use crate::wm::{App, Window};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct SettingsState {
    page: usize,           // 0 Display, 1 Sound, 2 System, 3 About
    pub volume: u32,       // 0..100
    pub username: String,
    pub test_sound: bool,  // one-shot request flag the WM consumes
}

const PAGES: [&str; 5] = ["Display", "Sound", "System", "Apps", "About"];
const BG: u32 = 0xff1a_1a1a;
const SIDE_BG: u32 = 0xff14_1414;
const ACCENT: u32 = 0xff5b_8af0;
const SIDE_W: isize = 120;

impl SettingsState {
    pub fn new() -> SettingsState {
        let mut s = SettingsState { page: 0, volume: 70, username: String::from("guest"), test_sound: false };
        s.load();
        s
    }

    fn load(&mut self) {
        if let Some(data) = crate::fs::read_file("SETTINGS.TXT") {
            for line in core::str::from_utf8(&data).unwrap_or("").lines() {
                if let Some((k, v)) = line.split_once('=') {
                    match k.trim() {
                        "volume" => self.volume = v.trim().parse().unwrap_or(70),
                        "username" => self.username = v.trim().to_string(),
                        _ => {}
                    }
                }
            }
        }
        if let Some(u) = crate::fs::read_file("USER.TXT") {
            if let Ok(s) = core::str::from_utf8(&u) {
                if !s.trim().is_empty() {
                    self.username = s.trim().to_string();
                }
            }
        }
    }

    fn save(&self) {
        let body = format!("volume={}\nusername={}\n", self.volume, self.username);
        let _ = crate::fs::write_file("SETTINGS.TXT", body.as_bytes());
    }
}

/// Returns true if a test-sound was requested (the WM should play it).
pub fn click(win: &mut Window, rx: isize, ry: isize) -> bool {
    let App::Settings(st) = &mut win.app else { return false };
    // Sidebar page selection.
    if rx < SIDE_W {
        let i = ((ry - 8) / 34) as usize;
        if i < PAGES.len() {
            st.page = i;
            render(win);
        }
        return false;
    }
    let px = rx - SIDE_W;
    let mut want_sound = false;
    match st.page {
        0 => {
            // Display: a "toggle wallpaper" button at y~60.
            if (40..76).contains(&ry) && (16..200).contains(&px) {
                crate::wm::toggle_wallpaper();
            }
        }
        1 => {
            // Sound: volume slider at y~70, x 16..280; test button below.
            if (60..86).contains(&ry) {
                let v = ((px - 16).max(0) * 100 / 264).min(100);
                st.volume = v as u32;
                st.save();
            } else if (110..150).contains(&ry) && (16..140).contains(&px) {
                want_sound = true;
                st.test_sound = true;
            }
        }
        3 => {
            // Apps & Permissions: a Revoke button per granted app.
            let cw = win.cw as isize;
            for (i, (name, _)) in crate::perms::all_grants().iter().enumerate() {
                let y = 78 + i as isize * 30;
                if (y - 4..y + 22).contains(&ry) && rx >= cw - 92 {
                    crate::perms::revoke(name, crate::perms::ALL);
                    break;
                }
            }
        }
        _ => {}
    }
    render(win);
    want_sound
}

pub fn render(win: &mut Window) {
    let App::Settings(st) = &win.app else { return };
    let (page, volume, username) = (st.page, st.volume, st.username.clone());
    let cw = win.cw;
    let ch = win.ch;
    let fb = win.canvas_fb();
    fb.clear(BG);
    // Sidebar.
    fb.fill_rect(0, 0, SIDE_W as usize, ch, SIDE_BG);
    for (i, name) in PAGES.iter().enumerate() {
        let y = 8 + i * 34;
        if i == page {
            fb.fill_round_rect(6, y, SIDE_W as usize - 12, 28, 6, ACCENT);
        }
        fb.draw_text(18, y + 6, name, FontId::Ui, 15, if i == page { 0xff0d0d0d } else { 0xffc8c8c8 });
    }
    let x0 = SIDE_W as usize + 16;
    fb.draw_text(x0, 12, PAGES[page], FontId::UiBold, 20, 0xfff0f0f0);
    match page {
        0 => {
            fb.draw_text(x0, 50, "Desktop background", FontId::Ui, 14, 0xffb0b0b0);
            fb.fill_round_rect(x0, 74, 168, 30, 6, 0xff2c2c2c);
            fb.draw_text(x0 + 16, 80, "Toggle Wallpaper", FontId::Ui, 14, 0xffe0e0e0);
        }
        1 => {
            fb.draw_text(x0, 50, &format!("Master volume: {volume}%"), FontId::Ui, 14, 0xffb0b0b0);
            let track_w = cw - x0 - 16;
            fb.fill_round_rect(x0, 72, track_w, 6, 3, 0xff3a3a3a);
            let fillw = track_w * volume as usize / 100;
            fb.fill_round_rect(x0, 72, fillw.max(4), 6, 3, ACCENT);
            fb.fill_circle((x0 + fillw) as isize, 75, 7, 0xffffffff);
            fb.fill_round_rect(x0, 110, 120, 32, 6, 0xff2c2c2c);
            fb.draw_text(x0 + 18, 117, "Test Sound", FontId::Ui, 14, 0xffe0e0e0);
        }
        2 => {
            let secs = crate::timer::wall_ticks50().map(|t| t / 50).unwrap_or_else(crate::timer::uptime_secs);
            fb.draw_text(x0, 50, &format!("Username: {username}"), FontId::Ui, 15, 0xffd0d0d0);
            fb.draw_text(x0, 78, &format!("Uptime: {:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60), FontId::Ui, 15, 0xffd0d0d0);
            fb.draw_text(x0, 106, &format!("Apps running: {}", crate::wm::window_count()), FontId::Ui, 15, 0xffd0d0d0);
        }
        3 => {
            fb.draw_text(x0, 50, "App permissions (third-party apps)", FontId::Ui, 14, 0xffb0b0b0);
            let grants = crate::perms::all_grants();
            if grants.is_empty() {
                fb.draw_text(x0, 80, "No app permissions granted yet.", FontId::Ui, 13, 0xff808080);
            }
            for (i, (name, bits)) in grants.iter().enumerate() {
                let y = 78 + i * 30;
                fb.draw_text(x0, y + 2, &format!("{name}", name = name), FontId::Ui, 14, 0xffe0e0e0);
                fb.draw_text(x0 + 130, y + 4, &crate::perms::list(*bits), FontId::Ui, 12, 0xffffd060);
                fb.fill_round_rect(cw - 92, y, 76, 22, 4, 0xff80_4040);
                fb.draw_text(cw - 80, y + 4, "Revoke", FontId::Ui, 12, 0xffffffff);
            }
        }
        _ => {
            let lines = [
                "Veil OS 0.36 — a from-scratch AArch64 OS",
                "",
                "Built by hand, no crates:",
                "  kernel, MMU, scheduler, FAT16",
                "  TCP/IP + TLS 1.3, HTTP browser",
                "  PNG/JPEG/GIF decoders, WASM JIT",
                "  FreeType2 (compiled from C source)",
                "  Lisp interpreter, virtio drivers",
            ];
            for (i, l) in lines.iter().enumerate() {
                fb.draw_text(x0, 50 + i * 22, l, FontId::Ui, 14, 0xffc0c4cc);
            }
        }
    }
}
