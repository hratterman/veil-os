//! Tiny 2D library over a linear XRGB8888 framebuffer: pixels, rects,
//! lines (Bresenham), and a bitmap font blitter.

use crate::font;
use crate::freetype::{self, FontId, GlyphBitmap};

#[derive(Clone, Copy)]
pub struct Framebuffer {
    base: *mut u32,
    pub width: usize,
    pub height: usize,
    stride_px: usize, // row pitch in pixels, not bytes
}

impl Framebuffer {
    /// # Safety
    /// `base` must point at `stride_bytes * height` of mapped memory.
    pub unsafe fn new(base: *mut u32, width: usize, height: usize, stride_bytes: usize) -> Self {
        Framebuffer {
            base,
            width,
            height,
            stride_px: stride_bytes / 4,
        }
    }

    #[inline]
    pub fn put_pixel(&self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            unsafe { self.base.add(y * self.stride_px + x).write_volatile(color) };
        }
    }

    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        if x < self.width && y < self.height {
            unsafe { self.base.add(y * self.stride_px + x).read_volatile() }
        } else {
            0
        }
    }

    // --- M35.6 FreeType anti-aliased text -----------------------------------

    /// Blit one FreeType glyph (8-bit alpha) at (gx, gy), alpha-blending each
    /// pixel against the framebuffer: out = fg*a/255 + dst*(255-a)/255.
    fn blit_glyph(&self, gx: i32, gy: i32, g: &GlyphBitmap, color: u32) {
        let (cr, cg, cb) = ((color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff);
        for row in 0..g.rows as i32 {
            let py = gy + row;
            if py < 0 || py as usize >= self.height {
                continue;
            }
            for col in 0..g.width as i32 {
                let px = gx + col;
                if px < 0 || px as usize >= self.width {
                    continue;
                }
                let a = g.data[(row as u32 * g.width + col as u32) as usize] as u32;
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    self.put_pixel(px as usize, py as usize, 0xff00_0000 | color);
                    continue;
                }
                let d = self.get_pixel(px as usize, py as usize);
                let bl = |s: u32, dc: u32| (s * a + dc * (255 - a)) / 255;
                let out = 0xff00_0000
                    | bl(cr, (d >> 16) & 0xff) << 16
                    | bl(cg, (d >> 8) & 0xff) << 8
                    | bl(cb, d & 0xff);
                self.put_pixel(px as usize, py as usize, out);
            }
        }
    }

    /// Draw `text` with FreeType at `size_px`, anti-aliased. `y` is the top of
    /// the line. Returns the advance width. Falls back to the 8x16 bitmap font
    /// before FreeType is initialised.
    pub fn draw_text(&self, x: usize, y: usize, text: &str, font: FontId, size_px: u16, color: u32) -> usize {
        if !freetype::ready() {
            self.draw_string(x, y, text, color, None);
            return text.chars().count() * 8;
        }
        let baseline = y as i32 + (size_px as i32 * 80) / 100; // ~ascender
        let mut pen = x as i32;
        for ch in text.chars() {
            crate::glyph_cache::with_glyph(font, ch, size_px, |g| match g {
                Some(g) => {
                    self.blit_glyph(pen + g.left, baseline - g.top, g, color);
                    pen += g.advance;
                }
                None => pen += (size_px / 3).max(2) as i32, // missing glyph / space
            });
        }
        (pen - x as i32).max(0) as usize
    }

    /// Pixel width/height of `text` at `size_px` (for layout).
    pub fn measure_text(&self, text: &str, font: FontId, size_px: u16) -> (usize, usize) {
        if !freetype::ready() {
            return (text.chars().count() * 8, 16);
        }
        let mut w = 0i32;
        for ch in text.chars() {
            crate::glyph_cache::with_glyph(font, ch, size_px, |g| {
                w += g.map(|g| g.advance).unwrap_or((size_px / 3).max(2) as i32);
            });
        }
        (w.max(0) as usize, size_px as usize)
    }

    pub fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for row in y..y1 {
            let mut p = unsafe { self.base.add(row * self.stride_px + x) };
            for _ in x..x1 {
                unsafe {
                    p.write_volatile(color);
                    p = p.add(1);
                }
            }
        }
    }

    pub fn clear(&self, color: u32) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    /// Filled rounded rectangle (fast interior fills + per-pixel corners).
    pub fn fill_round_rect(&self, x: usize, y: usize, w: usize, h: usize, radius: usize, color: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let r = radius.min(w / 2).min(h / 2);
        self.fill_rect(x, y + r, w, h - 2 * r, color);
        self.fill_rect(x + r, y, w - 2 * r, r, color);
        self.fill_rect(x + r, y + h - r, w - 2 * r, r, color);
        let r2 = (r * r) as isize;
        let corners = [
            (x + r, y + r, -1isize, -1isize),
            (x + w - 1 - r, y + r, 1, -1),
            (x + r, y + h - 1 - r, -1, 1),
            (x + w - 1 - r, y + h - 1 - r, 1, 1),
        ];
        for &(cx, cy, sx, sy) in &corners {
            for dy in 0..=r as isize {
                for dx in 0..=r as isize {
                    if dx * dx + dy * dy <= r2 {
                        self.put_pixel((cx as isize + sx * dx) as usize, (cy as isize + sy * dy) as usize, color);
                    }
                }
            }
        }
    }

    /// Knock the four corners of a rect out to `bg` (rounded-corner mask).
    pub fn round_corners(&self, x: usize, y: usize, w: usize, h: usize, radius: usize, bg: u32) {
        let r = radius.min(w / 2).min(h / 2);
        let r2 = (r * r) as isize;
        let corners = [
            (x + r, y + r, -1isize, -1isize),
            (x + w - 1 - r, y + r, 1, -1),
            (x + r, y + h - 1 - r, -1, 1),
            (x + w - 1 - r, y + h - 1 - r, 1, 1),
        ];
        for &(cx, cy, sx, sy) in &corners {
            for dy in 0..=r as isize {
                for dx in 0..=r as isize {
                    if dx * dx + dy * dy > r2 {
                        self.put_pixel((cx as isize + sx * dx) as usize, (cy as isize + sy * dy) as usize, bg);
                    }
                }
            }
        }
    }

    /// Filled circle centred at (cx, cy).
    pub fn fill_circle(&self, cx: isize, cy: isize, r: isize, color: u32) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let (px, py) = (cx + dx, cy + dy);
                    if px >= 0 && py >= 0 {
                        self.put_pixel(px as usize, py as usize, color);
                    }
                }
            }
        }
    }

    /// Alpha-blend a filled rect over the existing pixels. `alpha` is 0..=255
    /// (0 = invisible, 255 = opaque). Used for the semi-transparent icon that
    /// follows the cursor while a desktop icon is being dragged.
    pub fn blend_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u32, alpha: u32) {
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        let (cr, cg, cb) = ((color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff);
        let ia = 255 - alpha;
        for row in y..y1 {
            for col in x..x1 {
                let p = unsafe { self.base.add(row * self.stride_px + col) };
                let d = unsafe { p.read_volatile() };
                let r = (cr * alpha + ((d >> 16) & 0xff) * ia) / 255;
                let g = (cg * alpha + ((d >> 8) & 0xff) * ia) / 255;
                let b = (cb * alpha + (d & 0xff) * ia) / 255;
                unsafe { p.write_volatile(0xff00_0000 | (r << 16) | (g << 8) | b) };
            }
        }
    }

    pub fn draw_line(&self, x0: isize, y0: isize, x1: isize, y1: isize, color: u32) {
        let (mut x, mut y) = (x0, y0);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            if x >= 0 && y >= 0 {
                self.put_pixel(x as usize, y as usize, color);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Midpoint circle outline centered on (cx, cy). M19 clock faces.
    pub fn draw_circle(&self, cx: isize, cy: isize, r: isize, color: u32) {
        let put = |x: isize, y: isize| {
            if x >= 0 && y >= 0 {
                self.put_pixel(x as usize, y as usize, color);
            }
        };
        let (mut x, mut y) = (r, 0isize);
        let mut err = 1 - r;
        while x >= y {
            for &(px, py) in &[
                (cx + x, cy + y), (cx - x, cy + y), (cx + x, cy - y), (cx - x, cy - y),
                (cx + y, cy + x), (cx - y, cy + x), (cx + y, cy - x), (cx - y, cy - x),
            ] {
                put(px, py);
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Copy a `sw`x`sh` pixel buffer to (dx, dy), clipped to the screen.
    pub fn blit(&self, dx: isize, dy: isize, src: &[u32], sw: usize, sh: usize) {
        for row in 0..sh as isize {
            let y = dy + row;
            if y < 0 || y >= self.height as isize {
                continue;
            }
            let x0 = dx.max(0);
            let x1 = (dx + sw as isize).min(self.width as isize);
            if x0 >= x1 {
                continue;
            }
            let src_off = row as usize * sw + (x0 - dx) as usize;
            unsafe {
                let dst = self.base.add(y as usize * self.stride_px + x0 as usize);
                core::ptr::copy_nonoverlapping(src.as_ptr().add(src_off), dst, (x1 - x0) as usize);
            }
        }
    }

    /// Flip a full back buffer (stride == width) onto this framebuffer.
    pub fn copy_from(&self, src: &[u32]) {
        debug_assert!(self.stride_px == self.width && src.len() >= self.width * self.height);
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), self.base, self.width * self.height) };
    }

    /// Blit one glyph; `bg = None` leaves unlit pixels untouched.
    pub fn draw_char(&self, x: usize, y: usize, ch: u8, fg: u32, bg: Option<u32>) {
        let glyph_idx = ch.checked_sub(font::FIRST_CHAR).filter(|&i| (i as usize) < 95);
        let glyph = &font::GLYPHS[glyph_idx.unwrap_or(b'?' - font::FIRST_CHAR) as usize];
        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..font::FONT_WIDTH {
                if bits & (1 << col) != 0 {
                    self.put_pixel(x + col, y + row, fg);
                } else if let Some(bg) = bg {
                    self.put_pixel(x + col, y + row, bg);
                }
            }
        }
    }

    /// Like draw_char with each font pixel drawn as a scale x scale block.
    pub fn draw_char_scaled(&self, x: usize, y: usize, ch: u8, fg: u32, scale: usize) {
        if scale == 1 {
            return self.draw_char(x, y, ch, fg, None);
        }
        let glyph_idx = ch.checked_sub(font::FIRST_CHAR).filter(|&i| (i as usize) < 95);
        let glyph = &font::GLYPHS[glyph_idx.unwrap_or(b'?' - font::FIRST_CHAR) as usize];
        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..font::FONT_WIDTH {
                if bits & (1 << col) != 0 {
                    self.fill_rect(x + col * scale, y + row * scale, scale, scale, fg);
                }
            }
        }
    }

    pub fn draw_string_scaled(&self, x: usize, y: usize, s: &str, fg: u32, scale: usize) {
        let mut cx = x;
        for &b in s.as_bytes() {
            self.draw_char_scaled(cx, y, b, fg, scale);
            cx += font::FONT_WIDTH * scale;
        }
    }

    /// Blit one glyph from a generated `BitmapFont`; returns its advance.
    pub fn draw_bm_glyph(&self, x: usize, y: usize, font: &font::BitmapFont, ch: char, fg: u32) -> usize {
        let g = &font.glyphs[font::glyph_index(ch)];
        let (adv, w, off, len) = (g.0 as usize, g.1 as usize, g.2 as usize, g.3 as usize);
        let row_bytes = w.div_ceil(8);
        let bits = &font.bits[off..off + len];
        for r in 0..font.height as usize {
            for c in 0..w {
                if bits[r * row_bytes + (c >> 3)] & (0x80 >> (c & 7)) != 0 {
                    self.put_pixel(x + c, y + r, fg);
                }
            }
        }
        adv
    }

    /// Draw a string in a generated bitmap font (variable-width advances).
    pub fn draw_bm_string(&self, x: usize, y: usize, s: &str, font: &font::BitmapFont, fg: u32) {
        let mut px = x;
        for ch in s.chars() {
            px += self.draw_bm_glyph(px, y, font, ch, fg);
        }
    }

    pub fn draw_string(&self, x: usize, y: usize, s: &str, fg: u32, bg: Option<u32>) {
        let mut cx = x;
        let mut cy = y;
        for &b in s.as_bytes() {
            if b == b'\n' {
                cx = x;
                cy += font::FONT_HEIGHT;
                continue;
            }
            self.draw_char(cx, cy, b, fg, bg);
            cx += font::FONT_WIDTH;
        }
    }
}
