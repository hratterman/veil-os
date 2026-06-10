//! Tiny 2D library over a linear XRGB8888 framebuffer: pixels, rects,
//! lines (Bresenham), and a bitmap font blitter.

use crate::font;

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
