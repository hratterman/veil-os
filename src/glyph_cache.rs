//! M35.6 glyph cache: FreeType rendering is CPU-intensive, so each
//! (font, size, codepoint) is rasterised once and kept (LRU, 512 entries).
//! Lives on the kernel heap; glyphs are tiny 8-bit alpha bitmaps.

use crate::freetype::{render_glyph, FontId, GlyphBitmap};
use alloc::collections::BTreeMap;

type Key = (u8, u16, u32); // (font, size_px, codepoint)

struct Entry {
    glyph: Option<GlyphBitmap>, // None = glyph couldn't render (missing/fallback)
    used: u64,
}

struct Cache {
    map: BTreeMap<Key, Entry>,
    clock: u64,
}

static mut CACHE: Cache = Cache { map: BTreeMap::new(), clock: 0 };
const CAP: usize = 512;

fn fid(f: FontId) -> u8 {
    match f {
        FontId::Ui => 0,
        FontId::UiBold => 1,
        FontId::Mono => 2,
        FontId::Serif => 3,
    }
}

/// Pixel advance width of `s` at (font, size) — for layout without a framebuffer.
pub fn text_width(s: &str, font: FontId, size: u16) -> usize {
    let mut w = 0i32;
    for ch in s.chars() {
        with_glyph(font, ch, size, |g| {
            w += g.map(|g| g.advance).unwrap_or((size / 3).max(2) as i32)
        });
    }
    w.max(0) as usize
}

/// Borrow the cached (rasterising on first use) glyph for `cp`, passing it to
/// `f`. Avoids cloning the bitmap into every text draw.
pub fn with_glyph<R>(font: FontId, cp: char, size: u16, f: impl FnOnce(Option<&GlyphBitmap>) -> R) -> R {
    unsafe {
        let c = &mut *core::ptr::addr_of_mut!(CACHE);
        let key = (fid(font), size, cp as u32);
        c.clock += 1;
        if let Some(e) = c.map.get_mut(&key) {
            e.used = c.clock;
        } else {
            if c.map.len() >= CAP {
                // Evict the least-recently-used entry.
                if let Some(oldest) = c.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) {
                    c.map.remove(&oldest);
                }
            }
            let glyph = render_glyph(font, cp, size);
            c.map.insert(key, Entry { glyph, used: c.clock });
        }
        f(c.map.get(&key).and_then(|e| e.glyph.as_ref()))
    }
}
