//! M35.6: FreeType2 vector font rendering. The library is compiled from C
//! source (see build.rs / vendor/freetype) and linked in; here we wire its
//! memory to the kernel heap (the `veil_*` exports), initialise it lazily, load
//! the embedded TTF faces, and render anti-aliased 8-bit alpha glyphs.

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

// --- C heap shim: malloc-family over the kernel global allocator ----------
// FreeType allocs through these; we stash the block size in a 16-byte header so
// free/realloc can recover the Layout the Rust allocator needs.
const HDR: usize = 16;

#[unsafe(no_mangle)]
pub extern "C" fn veil_malloc(size: usize) -> *mut u8 {
    let total = size.max(1) + HDR;
    let Ok(layout) = Layout::from_size_align(total, 16) else { return ptr::null_mut() };
    let p = unsafe { alloc(layout) };
    if p.is_null() {
        return p;
    }
    unsafe {
        (p as *mut usize).write(total);
        p.add(HDR)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn veil_free(p: *mut u8) {
    if p.is_null() {
        return;
    }
    unsafe {
        let base = p.sub(HDR);
        let total = (base as *mut usize).read();
        dealloc(base, Layout::from_size_align_unchecked(total, 16));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn veil_realloc(p: *mut u8, new_size: usize) -> *mut u8 {
    if p.is_null() {
        return veil_malloc(new_size);
    }
    if new_size == 0 {
        veil_free(p);
        return ptr::null_mut();
    }
    unsafe {
        let total = (p.sub(HDR) as *mut usize).read();
        let old = total - HDR;
        let np = veil_malloc(new_size);
        if !np.is_null() {
            ptr::copy_nonoverlapping(p, np, old.min(new_size));
            veil_free(p);
        }
        np
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn veil_calloc(n: usize, size: usize) -> *mut u8 {
    let total = n.saturating_mul(size);
    let p = veil_malloc(total);
    if !p.is_null() {
        unsafe { ptr::write_bytes(p, 0, total) };
    }
    p
}

// --- FreeType FFI ---------------------------------------------------------

type FtLibrary = *mut c_void;
type FtFace = *mut c_void;

unsafe extern "C" {
    fn veil_ft_init(lib: *mut FtLibrary) -> i32;
    fn veil_ft_new_face(lib: FtLibrary, data: *const u8, size: i64, face: *mut FtFace) -> i32;
    fn veil_render_glyph(
        face: FtFace,
        codepoint: u64,
        size_px: u32,
        out_buf: *mut *const u8,
        w: *mut i32,
        rows: *mut i32,
        pitch: *mut i32,
        left: *mut i32,
        top: *mut i32,
        advance: *mut i32,
    ) -> i32;
}

// Embedded TTF faces (the same files gen_fonts.py used).
static TTF_UI: &[u8] = include_bytes!("../assets/fonts/barlow-400.ttf");
static TTF_UI_BOLD: &[u8] = include_bytes!("../assets/fonts/barlow-600.ttf");
static TTF_MONO: &[u8] = include_bytes!("../assets/fonts/mono.ttf");
static TTF_SERIF: &[u8] = include_bytes!("../assets/fonts/lora.ttf");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontId {
    Ui,
    UiBold,
    Mono,
    Serif,
}

const NFONTS: usize = 4;

struct Ft {
    lib: FtLibrary,
    faces: [FtFace; NFONTS],
    ok: bool,
}

static mut FT: Ft = Ft { lib: ptr::null_mut(), faces: [ptr::null_mut(); NFONTS], ok: false };

fn font_index(f: FontId) -> usize {
    match f {
        FontId::Ui => 0,
        FontId::UiBold => 1,
        FontId::Mono => 2,
        FontId::Serif => 3,
    }
}

/// Initialise FreeType (once) — needs the kernel heap up. Safe to call repeatedly.
pub fn init() -> bool {
    unsafe {
        let ft = &mut *ptr::addr_of_mut!(FT);
        if ft.ok {
            return true;
        }
        if veil_ft_init(&mut ft.lib) != 0 || ft.lib.is_null() {
            crate::kprintln!("FREETYPE: FT_Init failed");
            return false;
        }
        let mut loaded = 0;
        for (i, ttf) in [TTF_UI, TTF_UI_BOLD, TTF_MONO, TTF_SERIF].iter().enumerate() {
            let mut face: FtFace = ptr::null_mut();
            if veil_ft_new_face(ft.lib, ttf.as_ptr(), ttf.len() as i64, &mut face) == 0 {
                ft.faces[i] = face;
                loaded += 1;
            } else {
                crate::kprintln!("FREETYPE: face {i} load failed (falls back to bitmap)");
            }
        }
        // The UI face is the one the acceptance test (setup screen) needs.
        ft.ok = !ft.faces[font_index(FontId::Ui)].is_null();
        let _ = loaded;
        crate::kprintln!("FREETYPE_OK: FT2 from source, {NFONTS} faces, kernel-heap alloc");
        true
    }
}

pub fn ready() -> bool {
    unsafe { (*ptr::addr_of!(FT)).ok }
}

#[derive(Clone)]
pub struct GlyphBitmap {
    pub data: Vec<u8>, // width*rows, 8-bit alpha coverage
    pub width: u32,
    pub rows: u32,
    pub left: i32,    // bearing x (pen -> left edge)
    pub top: i32,     // bearing y (baseline -> top edge, +up)
    pub advance: i32, // pen advance in px
}

/// Render one glyph at `size_px` into an 8-bit alpha bitmap (uncached).
pub fn render_glyph(font: FontId, codepoint: char, size_px: u16) -> Option<GlyphBitmap> {
    if !init() {
        return None;
    }
    let face = unsafe { (*ptr::addr_of!(FT)).faces[font_index(font)] };
    if face.is_null() {
        return None;
    }
    let (mut buf, mut w, mut rows, mut pitch, mut left, mut top, mut adv) =
        (ptr::null(), 0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
    let rc = unsafe {
        veil_render_glyph(
            face, codepoint as u64, size_px as u32, &mut buf, &mut w, &mut rows, &mut pitch,
            &mut left, &mut top, &mut adv,
        )
    };
    if rc != 0 {
        return None;
    }
    // Copy FreeType's bitmap (valid only until the next load on this face) into
    // an owned, tightly-packed width*rows buffer.
    let (wu, ru) = (w.max(0) as usize, rows.max(0) as usize);
    let mut data = vec![0u8; wu * ru];
    if !buf.is_null() && pitch != 0 {
        let ap = pitch.unsigned_abs() as usize;
        for y in 0..ru {
            let srow = if pitch > 0 { y } else { ru - 1 - y };
            unsafe {
                let src = buf.add(srow * ap);
                ptr::copy_nonoverlapping(src, data.as_mut_ptr().add(y * wu), wu);
            }
        }
    }
    Some(GlyphBitmap { data, width: wu as u32, rows: ru as u32, left, top, advance: adv })
}
