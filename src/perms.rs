//! M41 step 16: capability-based security for WASM apps.
//!
//! Each app has a permission bitset. Host functions that touch a protected
//! resource (network / filesystem / audio / clipboard / notifications) check the
//! running app's permissions and reject the call if it isn't granted — the
//! *kernel* refuses, the app can't bypass it. Pre-bundled system apps get full
//! permissions; everything else starts with nothing and is granted via the
//! first-launch permission dialog (and revocable in Settings). Grants persist to
//! `PERMS.DAT` on the FAT16 disk.

use crate::fs;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const NETWORK: u32 = 1 << 0;
pub const FILESYSTEM: u32 = 1 << 1;
pub const AUDIO: u32 = 1 << 2;
pub const CLIPBOARD: u32 = 1 << 3;
pub const NOTIFY: u32 = 1 << 4;
pub const ALL: u32 = NETWORK | FILESYSTEM | AUDIO | CLIPBOARD | NOTIFY;

/// Apps that ship with the OS have implicit full permissions.
const SYSTEM_APPS: &[&str] = &["HELLOAPP.WSM", "NETGET.WSM", "EVIL.WSM", "HELLO.WSM", "COMPUTE.WSM"];

pub fn name(cap: u32) -> &'static str {
    match cap {
        NETWORK => "network",
        FILESYSTEM => "filesystem",
        AUDIO => "audio",
        CLIPBOARD => "clipboard",
        NOTIFY => "notifications",
        _ => "?",
    }
}

/// Render a permission bitset as "network, filesystem".
pub fn list(bits: u32) -> String {
    let mut out = Vec::new();
    for c in [NETWORK, FILESYSTEM, AUDIO, CLIPBOARD, NOTIFY] {
        if bits & c != 0 {
            out.push(name(c));
        }
    }
    out.join(", ")
}

// Per-app grants, keyed by the app's filename (e.g. "APP1.WSM").
static mut GRANTS: Option<BTreeMap<String, u32>> = None;

fn grants() -> &'static mut BTreeMap<String, u32> {
    unsafe {
        let g = &mut *core::ptr::addr_of_mut!(GRANTS);
        if g.is_none() {
            *g = Some(load());
        }
        g.as_mut().unwrap()
    }
}

pub fn is_system(app: &str) -> bool {
    SYSTEM_APPS.iter().any(|s| s.eq_ignore_ascii_case(app))
}

/// The capabilities currently granted to `app` (full for system apps).
pub fn for_app(app: &str) -> u32 {
    if is_system(app) {
        return ALL;
    }
    grants().get(app).copied().unwrap_or(0)
}

pub fn grant(app: &str, bits: u32) {
    let g = grants();
    let e = g.entry(app.to_string()).or_insert(0);
    *e |= bits;
    crate::kprintln!("PERMS: granted {} to {app}", list(bits));
    save();
}

pub fn revoke(app: &str, bits: u32) {
    if let Some(e) = grants().get_mut(app) {
        *e &= !bits;
        crate::kprintln!("PERMS: revoked {} from {app}", list(bits));
        save();
    }
}

/// True if `app` has every capability in `bits`.
pub fn has(app: &str, bits: u32) -> bool {
    for_app(app) & bits == bits
}

/// Apps with a recorded grant, for the Settings panel.
pub fn all_grants() -> Vec<(String, u32)> {
    grants().iter().map(|(k, v)| (k.clone(), *v)).collect()
}

fn load() -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    if let Some(d) = fs::read_file("PERMS.DAT") {
        for line in String::from_utf8_lossy(&d).lines() {
            if let Some((name, bits)) = line.split_once('\t') {
                if let Ok(b) = bits.trim().parse::<u32>() {
                    m.insert(name.to_string(), b);
                }
            }
        }
    }
    m
}

fn save() {
    let g = grants();
    let mut s = String::new();
    for (k, v) in g.iter() {
        s.push_str(k);
        s.push('\t');
        s.push_str(&alloc::format!("{v}\n"));
    }
    let _ = fs::write_file("PERMS.DAT", s.as_bytes());
}
