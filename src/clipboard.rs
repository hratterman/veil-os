//! M35 global clipboard: a single kernel-wide text buffer that Ctrl+C fills
//! and Ctrl+V reads. Rendering and input run single-threaded on the desktop
//! task, so a plain static is enough.

use alloc::string::String;

static mut CLIP: Option<String> = None;

pub fn set(s: String) {
    let n = s.len();
    unsafe { *core::ptr::addr_of_mut!(CLIP) = Some(s) };
    crate::kprintln!("CLIPBOARD: copied {n} bytes");
}

pub fn get() -> String {
    unsafe { (*core::ptr::addr_of!(CLIP)).clone().unwrap_or_default() }
}

pub fn is_empty() -> bool {
    unsafe { (*core::ptr::addr_of!(CLIP)).as_ref().is_none_or(String::is_empty) }
}
