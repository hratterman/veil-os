//! M41 step 13: the in-OS app store / loader (App::Store).
//!
//! Lists the installed WASM apps (`*.WSM` on the FAT16 disk) and lets the user
//! **install a new one from a URL**: paste a `.wasm` URL, press Enter, and the
//! store fetches it over the kernel HTTP/TLS stack, validates the magic, and
//! saves it as `APPn.WSM`. Clicking an app runs it (the graphical WASM runtime).
//! An optional `veil.toml` manifest (name/version/permissions) is parsed if the
//! download is a manifest that points at a `wasm =` URL.

use crate::fb::Framebuffer;
use crate::wm::{App, Window};
use crate::{fs, kprintln};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BG: u32 = 0xff10_141a;
const TITLE: u32 = 0xff5b_8af0;
const TEXT: u32 = 0xffd0_d8e0;
const MUTED: u32 = 0xff80_8a96;
const FIELD_BG: u32 = 0xff1c_2430;
const SEL_BG: u32 = 0xff2a_5a8a;
const BTN_BG: u32 = 0xff2f_9e6b;
const ROW_H: usize = 18;
const LIST_TOP: usize = 96;

const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_ENTER: u16 = 28;
const KEY_BACKSPACE: u16 = 14;

pub struct StoreState {
    apps: Vec<String>, // installed .WSM files
    url: String,
    editing: bool,
    status: String,
    sel: usize,
}

pub enum Action {
    None,
    Redraw,
    Run(String),
}

impl StoreState {
    pub fn new() -> StoreState {
        let mut s = StoreState {
            apps: Vec::new(),
            url: String::from("/HELLOAPP.WSM"),
            editing: false,
            status: String::from("Paste a .wasm URL and press Enter to install."),
            sel: 0,
        };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        self.apps = fs::list_root()
            .unwrap_or_default()
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".WSM"))
            .collect();
        self.apps.sort();
        kprintln!("STORE: {} app(s) installed", self.apps.len());
    }

    /// Install the app at `url`: fetch, validate, save under a fresh name.
    fn install(&mut self) {
        let url = self.url.trim().to_string();
        if url.is_empty() {
            return;
        }
        kprintln!("STORE: installing from {url}");
        let Some((status, body)) = crate::browser::shell_fetch(&url, None) else {
            self.status = format!("install failed: no response from {url}");
            kprintln!("STORE: install failed (no response)");
            return;
        };
        // A veil.toml manifest? Parse it and follow its `wasm =` URL.
        let (bytes, manifest) = if looks_like_toml(&body) {
            let m = parse_manifest(&String::from_utf8_lossy(&body));
            match m.wasm.as_ref().and_then(|w| crate::browser::shell_fetch(w, None)) {
                Some((_, b)) => (b, Some(m)),
                None => {
                    self.status = String::from("manifest had no reachable wasm = URL");
                    return;
                }
            }
        } else {
            (body, None)
        };
        if status != 200 || !bytes.starts_with(b"\0asm") {
            self.status = format!("not a WASM app (status {status})");
            kprintln!("STORE: install rejected (status {status}, magic ok={})", bytes.starts_with(b"\0asm"));
            return;
        }
        let name = self.fresh_name(manifest.as_ref());
        match fs::write_file(&name, &bytes) {
            Ok(()) => {
                let label = manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| name.clone());
                self.status = format!("installed {label} -> {name} ({} bytes)", bytes.len());
                kprintln!("STORE: installed {name} ({} bytes) from {url}", bytes.len());
                kprintln!("STORE_INSTALL_OK: {name}");
                if let Some(m) = &manifest {
                    kprintln!("STORE: manifest name='{}' version='{}' perms={:?}", m.name, m.version, m.perms);
                }
                self.refresh();
                if let Some(i) = self.apps.iter().position(|a| a == &name) {
                    self.sel = i;
                }
            }
            Err(()) => {
                self.status = format!("could not write {name} (disk full?)");
                kprintln!("STORE: write {name} failed");
            }
        }
    }

    /// A free `APPn.WSM` name (or the manifest's name, 8.3-clamped).
    fn fresh_name(&self, manifest: Option<&Manifest>) -> String {
        if let Some(m) = manifest {
            let stem: String = m.name.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>().to_ascii_uppercase();
            if !stem.is_empty() {
                let n = format!("{stem}.WSM");
                if !self.apps.contains(&n) {
                    return n;
                }
            }
        }
        for i in 1..1000 {
            let n = format!("APP{i}.WSM");
            if !self.apps.contains(&n) {
                return n;
            }
        }
        String::from("APP.WSM")
    }
}

/// Manifest parsed from a `veil.toml`.
#[derive(Clone)]
struct Manifest {
    name: String,
    version: String,
    perms: Vec<String>,
    wasm: Option<String>,
}

fn looks_like_toml(body: &[u8]) -> bool {
    !body.starts_with(b"\0asm")
        && core::str::from_utf8(body).map(|s| s.contains("name") && (s.contains("wasm") || s.contains("[app]"))).unwrap_or(false)
}

/// Minimal TOML subset: `key = "value"` and `permissions = ["a", "b"]`.
fn parse_manifest(src: &str) -> Manifest {
    let mut m = Manifest { name: String::from("app"), version: String::from("0.0"), perms: Vec::new(), wasm: None };
    for line in src.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let unq = v.trim_matches(|c| c == '"' || c == '\'');
        match k {
            "name" => m.name = unq.to_string(),
            "version" => m.version = unq.to_string(),
            "wasm" | "url" => m.wasm = Some(unq.to_string()),
            "permissions" => {
                m.perms = v.trim_matches(|c| c == '[' || c == ']').split(',')
                    .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    m
}

pub fn render(win: &mut Window) {
    let (apps, url, editing, status, sel) = {
        let App::Store(st) = &win.app else { return };
        (st.apps.clone(), st.url.clone(), st.editing, st.status.clone(), st.sel)
    };
    let (cw, ch) = (win.cw, win.ch);
    let fb = win.canvas_fb();
    fb.clear(BG);
    fb.draw_text(12, 8, "Veil App Store", crate::freetype::FontId::Ui, 22, TITLE);
    fb.draw_text(12, 42, "Install from URL:", crate::freetype::FontId::Ui, 14, MUTED);
    // URL field
    fb.fill_rect(12, 60, cw.saturating_sub(110), 22, FIELD_BG);
    let shown = if editing { format!("{url}_") } else { url.clone() };
    fb.draw_text(18, 62, &shown, crate::freetype::FontId::Ui, 14, TEXT);
    // Install button
    fb.fill_round_rect(cw.saturating_sub(92), 60, 80, 22, 4, BTN_BG);
    fb.draw_text(cw.saturating_sub(78), 62, "Install", crate::freetype::FontId::Ui, 14, 0xffffffff);
    // App list
    fb.draw_text(12, LIST_TOP - 18, "Installed apps (click to run):", crate::freetype::FontId::Ui, 13, MUTED);
    for (i, app) in apps.iter().enumerate() {
        let y = LIST_TOP + i * ROW_H;
        if y + ROW_H > ch.saturating_sub(20) {
            break;
        }
        if i == sel {
            fb.fill_rect(8, y, cw.saturating_sub(16), ROW_H, SEL_BG);
        }
        let col = if i == sel { 0xffff_ffff } else { TEXT };
        fb.draw_text(16, y + 1, app, crate::freetype::FontId::Ui, 14, col);
    }
    // Status bar
    fb.fill_rect(0, ch.saturating_sub(18), cw, 18, 0xff1a_222c);
    fb.draw_text(8, ch.saturating_sub(17), &status, crate::freetype::FontId::Ui, 12, MUTED);
}

pub fn click(win: &mut Window, rx: isize, ry: isize) -> Action {
    let (cw, n) = {
        let App::Store(st) = &win.app else { return Action::None };
        (win.cw as isize, st.apps.len())
    };
    // URL field
    if ry >= 60 && ry < 82 && rx >= 12 && rx < cw - 100 {
        if let App::Store(st) = &mut win.app {
            st.editing = true;
        }
        return Action::Redraw;
    }
    // Install button
    if ry >= 60 && ry < 82 && rx >= cw - 92 {
        if let App::Store(st) = &mut win.app {
            st.install();
        }
        return Action::Redraw;
    }
    // App row
    if ry >= LIST_TOP as isize {
        let i = ((ry - LIST_TOP as isize) / ROW_H as isize) as usize;
        if i < n {
            let name = {
                let App::Store(st) = &mut win.app else { return Action::None };
                st.sel = i;
                st.editing = false;
                st.apps[i].clone()
            };
            kprintln!("STORE: run {name}");
            return Action::Run(name);
        }
    }
    Action::None
}

pub fn key(win: &mut Window, code: u16) -> Action {
    let App::Store(st) = &mut win.app else { return Action::None };
    match code {
        KEY_ENTER if st.editing => {
            st.install();
            st.editing = false;
            Action::Redraw
        }
        KEY_ENTER => {
            if let Some(name) = st.apps.get(st.sel).cloned() {
                return Action::Run(name);
            }
            Action::None
        }
        KEY_BACKSPACE if st.editing => {
            st.url.pop();
            Action::Redraw
        }
        KEY_UP => {
            st.sel = st.sel.saturating_sub(1);
            Action::Redraw
        }
        KEY_DOWN => {
            if st.sel + 1 < st.apps.len() {
                st.sel += 1;
            }
            Action::Redraw
        }
        _ => Action::None,
    }
}

pub fn char_input(win: &mut Window, ch: char) {
    let App::Store(st) = &mut win.app else { return };
    if st.editing && !ch.is_control() {
        st.url.push(ch);
    }
}
