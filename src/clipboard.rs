//! M35 global clipboard: a single kernel-wide text buffer that Ctrl+C fills
//! and Ctrl+V reads. Rendering and input run single-threaded on the desktop
//! task, so a plain static is enough.

use alloc::string::String;
use alloc::vec::Vec;

static mut CLIP: Option<String> = None;
// M36: history of the last 10 distinct copies (newest first).
static mut HISTORY: Vec<String> = Vec::new();

pub fn set(s: String) {
    let n = s.len();
    if !s.is_empty() {
        let hist = unsafe { &mut *core::ptr::addr_of_mut!(HISTORY) };
        hist.retain(|e| e != &s);
        hist.insert(0, s.clone());
        hist.truncate(10);
    }
    unsafe { *core::ptr::addr_of_mut!(CLIP) = Some(s) };
    crate::kprintln!("CLIPBOARD: copied {n} bytes");
}

pub fn get() -> String {
    unsafe { (*core::ptr::addr_of!(CLIP)).clone().unwrap_or_default() }
}

pub fn is_empty() -> bool {
    unsafe { (*core::ptr::addr_of!(CLIP)).as_ref().is_none_or(String::is_empty) }
}

/// The clipboard history, newest first (for the Ctrl+Shift+V picker).
pub fn history() -> Vec<String> {
    unsafe { (*core::ptr::addr_of!(HISTORY)).clone() }
}

/// Promote history entry `i` to the active clipboard.
pub fn pick(i: usize) {
    let hist = unsafe { &*core::ptr::addr_of!(HISTORY) };
    if let Some(s) = hist.get(i).cloned() {
        unsafe { *core::ptr::addr_of_mut!(CLIP) = Some(s) };
        crate::kprintln!("CLIPBOARD: picked history #{i}");
    }
}
