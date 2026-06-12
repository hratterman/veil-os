//! A from-scratch HTML5 `<canvas>` 2D rendering context for the JS engine.
//!
//! Each `getContext('2d')` call binds to a `Canvas` here (an ARGB pixel buffer
//! plus the usual context state: fill/stroke style, lineWidth, globalAlpha, a
//! 2x3 affine transform, a save/restore stack and the current path). Drawing
//! methods rasterize straight into the buffer (source-over alpha blend). After
//! the page's scripts run, the browser flattens each buffer over white and
//! composites it where the `<canvas>` element sits in layout — so a Canvas
//! charting library draws a real chart inside our static layout engine.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::freetype::FontId;
use super::mathf;

// core lacks f64 methods in no_std; route through the AArch64 mathf intrinsics.
#[inline] fn sqrt(x: f64) -> f64 { mathf::sqrt(x) }
#[inline] fn floor(x: f64) -> f64 { mathf::floor(x) }
#[inline] fn ceil(x: f64) -> f64 { mathf::ceil(x) }
#[inline] fn round(x: f64) -> f64 { mathf::floor(x + 0.5) }

#[derive(Clone, Copy)]
struct Affine {
    a: f64, b: f64, c: f64, d: f64, e: f64, f: f64,
}

impl Affine {
    fn id() -> Affine { Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 } }
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
    fn mul(&self, o: Affine) -> Affine {
        // self * o  (apply o first, then self)
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    /// Mean axis scale, for stroke width / arc flattening in device space.
    fn scale(&self) -> f64 {
        let sx = sqrt(self.a * self.a + self.b * self.b);
        let sy = sqrt(self.c * self.c + self.d * self.d);
        ((sx + sy) / 2.0).max(0.01)
    }
}

#[derive(Clone)]
struct State {
    fill: u32,   // straight ARGB
    stroke: u32,
    line_w: f64,
    alpha: f64,  // globalAlpha 0..1
    font_px: u16,
    font_id: FontId,
    tf: Affine,
}

impl State {
    fn new() -> State {
        State {
            fill: 0xff00_0000,
            stroke: 0xff00_0000,
            line_w: 1.0,
            alpha: 1.0,
            font_px: 13,
            font_id: FontId::Ui,
            tf: Affine::id(),
        }
    }
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    px: Vec<u32>, // ARGB straight-alpha; 0 = transparent
    st: State,
    stack: Vec<State>,
    subpaths: Vec<Vec<(f64, f64)>>, // device-space points
    closed: Vec<bool>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        let (w, h) = (w.clamp(1, 2048), h.clamp(1, 2048));
        Canvas {
            w, h,
            px: vec![0u32; w * h],
            st: State::new(),
            stack: Vec::new(),
            subpaths: Vec::new(),
            closed: Vec::new(),
        }
    }

    /// Flatten the ARGB buffer over an opaque white page into XRGB for the
    /// browser's blit path.
    pub fn flatten(&self) -> Vec<u32> {
        self.px.iter().map(|&p| over_white(p)).collect()
    }

    /// A clone of the raw ARGB buffer (for canvas->canvas drawImage).
    pub fn snapshot(&self) -> (Vec<u32>, usize, usize) {
        (self.px.clone(), self.w, self.h)
    }

    // --- state / style -----------------------------------------------------

    pub fn set_prop(&mut self, prop: &str, val: &str) {
        match prop {
            "fillStyle" => self.st.fill = parse_color(val).unwrap_or(self.st.fill),
            "strokeStyle" => self.st.stroke = parse_color(val).unwrap_or(self.st.stroke),
            "lineWidth" => self.st.line_w = val.trim().parse::<f64>().unwrap_or(self.st.line_w).max(0.1),
            "globalAlpha" => self.st.alpha = val.trim().parse::<f64>().unwrap_or(self.st.alpha).clamp(0.0, 1.0),
            "font" => self.set_font(val),
            _ => {}
        }
    }

    pub fn get_prop(&self, prop: &str) -> Option<String> {
        match prop {
            "fillStyle" => Some(color_str(self.st.fill)),
            "strokeStyle" => Some(color_str(self.st.stroke)),
            "lineWidth" => Some(num_str(self.st.line_w)),
            "globalAlpha" => Some(num_str(self.st.alpha)),
            "width" => Some(num_str(self.w as f64)),
            "height" => Some(num_str(self.h as f64)),
            _ => None,
        }
    }

    fn set_font(&mut self, font: &str) {
        // Parse the px size out of e.g. "bold 16px Arial".
        let mut px = self.st.font_px;
        for tok in font.split_whitespace() {
            if let Some(n) = tok.strip_suffix("px") {
                if let Ok(v) = n.parse::<f64>() {
                    px = round(v).clamp(6.0, 96.0) as u16;
                }
            }
        }
        self.st.font_px = px;
        let low = font.to_ascii_lowercase();
        self.st.font_id = if low.contains("mono") || low.contains("courier") {
            FontId::Mono
        } else {
            FontId::Ui
        };
    }

    pub fn save(&mut self) { self.stack.push(self.st.clone()); }
    pub fn restore(&mut self) { if let Some(s) = self.stack.pop() { self.st = s; } }

    pub fn translate(&mut self, x: f64, y: f64) {
        self.st.tf = self.st.tf.mul(Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: x, f: y });
    }
    pub fn scale(&mut self, x: f64, y: f64) {
        self.st.tf = self.st.tf.mul(Affine { a: x, b: 0.0, c: 0.0, d: y, e: 0.0, f: 0.0 });
    }
    pub fn rotate(&mut self, ang: f64) {
        let (s, c) = (sin(ang), cos(ang));
        self.st.tf = self.st.tf.mul(Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 });
    }
    pub fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        self.st.tf = Affine { a, b, c, d, e, f };
    }
    pub fn transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        self.st.tf = self.st.tf.mul(Affine { a, b, c, d, e, f });
    }
    pub fn reset_transform(&mut self) { self.st.tf = Affine::id(); }

    // --- rectangles --------------------------------------------------------

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let col = self.st.fill;
        self.fill_quad(x, y, w, h, col);
    }
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        // Clear to transparent (axis-aligned in device space — the common case).
        let (x0, y0) = self.st.tf.apply(x, y);
        let (x1, y1) = self.st.tf.apply(x + w, y + h);
        let (lx, hx) = (x0.min(x1) as isize, x0.max(x1) as isize);
        let (ly, hy) = (y0.min(y1) as isize, y0.max(y1) as isize);
        for py in ly.max(0)..hy.min(self.h as isize) {
            for px in lx.max(0)..hx.min(self.w as isize) {
                self.px[py as usize * self.w + px as usize] = 0;
            }
        }
    }
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.begin_path();
        self.rect(x, y, w, h);
        self.closed.last_mut().map(|c| *c = true);
        self.stroke();
    }

    fn fill_quad(&mut self, x: f64, y: f64, w: f64, h: f64, col: u32) {
        let pts = [
            self.st.tf.apply(x, y),
            self.st.tf.apply(x + w, y),
            self.st.tf.apply(x + w, y + h),
            self.st.tf.apply(x, y + h),
        ];
        self.fill_polys(&[pts.to_vec()], col);
    }

    // --- paths -------------------------------------------------------------

    pub fn begin_path(&mut self) {
        self.subpaths.clear();
        self.closed.clear();
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.subpaths.push(vec![self.st.tf.apply(x, y)]);
        self.closed.push(false);
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        if self.subpaths.is_empty() {
            self.move_to(x, y);
        } else {
            let p = self.st.tf.apply(x, y);
            self.subpaths.last_mut().unwrap().push(p);
        }
    }
    pub fn close_path(&mut self) {
        if let Some(c) = self.closed.last_mut() {
            *c = true;
        }
    }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
    }
    pub fn arc(&mut self, cx: f64, cy: f64, r: f64, a0: f64, a1: f64, ccw: bool) {
        let dscale = self.st.tf.scale();
        let segs = ((r * dscale).abs() as usize / 3 + 8).min(180);
        let mut a1 = a1;
        if !ccw && a1 < a0 { a1 += 2.0 * core::f64::consts::PI; }
        if ccw && a1 > a0 { a1 -= 2.0 * core::f64::consts::PI; }
        for i in 0..=segs {
            let t = a0 + (a1 - a0) * (i as f64 / segs as f64);
            let (x, y) = (cx + r * cos(t), cy + r * sin(t));
            if i == 0 && self.subpaths.last().map_or(true, |s| s.is_empty()) {
                self.line_to(x, y);
            } else if i == 0 {
                self.line_to(x, y);
            } else {
                self.line_to(x, y);
            }
        }
    }
    pub fn arc_to(&mut self, x1: f64, y1: f64, _x2: f64, _y2: f64, _r: f64) {
        // Approximate: just line to the first control point.
        self.line_to(x1, y1);
    }
    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let start = self.cur_point();
        let (x0, y0) = start;
        let segs = 24;
        for i in 1..=segs {
            let t = i as f64 / segs as f64;
            let mt = 1.0 - t;
            let bx = mt*mt*mt*x0 + 3.0*mt*mt*t*c1x + 3.0*mt*t*t*c2x + t*t*t*x;
            let by = mt*mt*mt*y0 + 3.0*mt*mt*t*c1y + 3.0*mt*t*t*c2y + t*t*t*y;
            self.line_to(bx, by);
        }
    }
    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let (x0, y0) = self.cur_point();
        let segs = 20;
        for i in 1..=segs {
            let t = i as f64 / segs as f64;
            let mt = 1.0 - t;
            let bx = mt*mt*x0 + 2.0*mt*t*cx + t*t*x;
            let by = mt*mt*y0 + 2.0*mt*t*cy + t*t*y;
            self.line_to(bx, by);
        }
    }
    /// Current path point in *user* space (inverse-transform the last device pt).
    fn cur_point(&self) -> (f64, f64) {
        // We only need it to seed bezier in user space; invert the last device pt.
        if let Some(last) = self.subpaths.last().and_then(|s| s.last()) {
            inv_apply(&self.st.tf, last.0, last.1)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn fill(&mut self) {
        let col = self.st.fill;
        let polys = self.subpaths.clone();
        self.fill_polys(&polys, col);
    }
    pub fn stroke(&mut self) {
        let col = self.st.stroke;
        let lw = (self.st.line_w * self.st.tf.scale()).max(1.0);
        let paths = self.subpaths.clone();
        let closed = self.closed.clone();
        for (i, sp) in paths.iter().enumerate() {
            if sp.len() < 2 {
                continue;
            }
            for w in sp.windows(2) {
                self.thick_line(w[0], w[1], lw, col);
            }
            if closed.get(i).copied().unwrap_or(false) {
                self.thick_line(sp[sp.len() - 1], sp[0], lw, col);
            }
        }
    }

    // --- text --------------------------------------------------------------

    pub fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        let (dx, dy) = self.st.tf.apply(x, y);
        // y is the text baseline; FreeType draw_text takes a top-left-ish origin,
        // so lift by ~the ascent (≈0.8 of px).
        let top = dy - self.st.font_px as f64 * 0.8;
        let col = with_alpha(self.st.fill, self.st.alpha);
        // Wrap our buffer in a Framebuffer and blend the glyphs in.
        let fb = unsafe {
            crate::fb::Framebuffer::new(self.px.as_mut_ptr(), self.w, self.h, self.w * 4)
        };
        if top >= -(self.st.font_px as f64) && dx < self.w as f64 && top < self.h as f64 {
            fb.draw_text(dx.max(0.0) as usize, top.max(0.0) as usize, text, self.st.font_id, self.st.font_px, col | 0xff00_0000);
        }
    }
    pub fn stroke_text(&mut self, text: &str, x: f64, y: f64) {
        // Approximate stroked text as filled in the stroke color.
        let save = self.st.fill;
        self.st.fill = self.st.stroke;
        self.fill_text(text, x, y);
        self.st.fill = save;
    }
    /// Returns the advance width in CSS px (transform scale not applied).
    pub fn measure_text(&self, text: &str) -> f64 {
        crate::glyph_cache::text_width(text, self.st.font_id, self.st.font_px) as f64
    }

    // --- images (canvas -> canvas only; <img> pixels live in the browser) --

    pub fn draw_image_buf(&mut self, src: &[u32], sw: usize, sh: usize, dx: f64, dy: f64, dw: f64, dh: f64) {
        if sw == 0 || sh == 0 {
            return;
        }
        let (ox, oy) = self.st.tf.apply(dx, dy);
        let dwp = (dw * self.st.tf.scale()).max(1.0);
        let dhp = (dh * self.st.tf.scale()).max(1.0);
        for py in 0..dhp as isize {
            let ty = oy as isize + py;
            if ty < 0 || ty >= self.h as isize { continue; }
            let syf = py as f64 / dhp * sh as f64;
            for px in 0..dwp as isize {
                let tx = ox as isize + px;
                if tx < 0 || tx >= self.w as isize { continue; }
                let sxf = px as f64 / dwp * sw as f64;
                let sp = src[(syf as usize).min(sh - 1) * sw + (sxf as usize).min(sw - 1)];
                self.blend_px(tx as usize, ty as usize, with_alpha(sp, self.st.alpha));
            }
        }
    }

    // --- pixel data --------------------------------------------------------

    /// (rgba bytes, w, h) for a region, clamped to the canvas.
    pub fn get_image_data(&self, x: f64, y: f64, w: f64, h: f64) -> (Vec<u8>, usize, usize) {
        let (rw, rh) = ((w.max(0.0) as usize).min(self.w), (h.max(0.0) as usize).min(self.h));
        let (ox, oy) = (x.max(0.0) as usize, y.max(0.0) as usize);
        let mut out = Vec::with_capacity(rw * rh * 4);
        for j in 0..rh {
            for i in 0..rw {
                let p = self.px.get((oy + j) * self.w + (ox + i)).copied().unwrap_or(0);
                out.push(((p >> 16) & 0xff) as u8);
                out.push(((p >> 8) & 0xff) as u8);
                out.push((p & 0xff) as u8);
                out.push(((p >> 24) & 0xff) as u8);
            }
        }
        (out, rw, rh)
    }
    pub fn put_image_data(&mut self, data: &[u8], dw: usize, dh: usize, dx: f64, dy: f64) {
        let (ox, oy) = (dx as isize, dy as isize);
        for j in 0..dh {
            for i in 0..dw {
                let k = (j * dw + i) * 4;
                if k + 3 >= data.len() { break; }
                let (tx, ty) = (ox + i as isize, oy + j as isize);
                if tx < 0 || ty < 0 || tx >= self.w as isize || ty >= self.h as isize { continue; }
                let p = (data[k + 3] as u32) << 24 | (data[k] as u32) << 16 | (data[k + 1] as u32) << 8 | data[k + 2] as u32;
                self.px[ty as usize * self.w + tx as usize] = p; // putImageData overwrites
            }
        }
    }

    // --- rasterization core ------------------------------------------------

    fn fill_polys(&mut self, polys: &[Vec<(f64, f64)>], col: u32) {
        let col = with_alpha(col, self.st.alpha);
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for p in polys {
            for &(_, y) in p {
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
        if !ymin.is_finite() {
            return;
        }
        let y0 = (floor(ymin) as isize).max(0);
        let y1 = (ceil(ymax) as isize).min(self.h as isize);
        let mut xs: Vec<f64> = Vec::new();
        for sy in y0..y1 {
            let yc = sy as f64 + 0.5;
            xs.clear();
            for poly in polys {
                let n = poly.len();
                if n < 2 { continue; }
                for i in 0..n {
                    let (ax, ay) = poly[i];
                    let (bx, by) = poly[(i + 1) % n];
                    if (ay <= yc && by > yc) || (by <= yc && ay > yc) {
                        let t = (yc - ay) / (by - ay);
                        xs.push(ax + t * (bx - ax));
                    }
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < xs.len() {
                let (l, r) = (xs[i], xs[i + 1]);
                let lx = (ceil(l) as isize).max(0);
                let rx = (floor(r) as isize).min(self.w as isize - 1);
                for px in lx..=rx {
                    self.blend_px(px as usize, sy as usize, col);
                }
                i += 2;
            }
        }
    }

    fn thick_line(&mut self, a: (f64, f64), b: (f64, f64), w: f64, col: u32) {
        let col = with_alpha(col, self.st.alpha);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = sqrt(dx * dx + dy * dy).max(0.0001);
        // Build a rotated rectangle (quad) of width w around the segment.
        let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
        let quad = vec![
            (a.0 + nx, a.1 + ny),
            (b.0 + nx, b.1 + ny),
            (b.0 - nx, b.1 - ny),
            (a.0 - nx, a.1 - ny),
        ];
        // Fill the quad directly (single subpath, no globalAlpha re-multiply).
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &(_, y) in &quad {
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        let y0 = (floor(ymin) as isize).max(0);
        let y1 = (ceil(ymax) as isize).min(self.h as isize);
        let mut xs: Vec<f64> = Vec::new();
        for sy in y0..y1 {
            let yc = sy as f64 + 0.5;
            xs.clear();
            for i in 0..4 {
                let (ax, ay) = quad[i];
                let (bx, by) = quad[(i + 1) % 4];
                if (ay <= yc && by > yc) || (by <= yc && ay > yc) {
                    let t = (yc - ay) / (by - ay);
                    xs.push(ax + t * (bx - ax));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < xs.len() {
                let lx = (floor(xs[i]) as isize).max(0);
                let rx = (ceil(xs[i + 1]) as isize).min(self.w as isize - 1);
                for px in lx..=rx {
                    self.blend_px(px as usize, sy as usize, col);
                }
                i += 2;
            }
        }
    }

    fn blend_px(&mut self, x: usize, y: usize, src: u32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = y * self.w + x;
        self.px[i] = blend_over(self.px[i], src);
    }
}

// --- color + blend helpers ----------------------------------------------------

/// Source-over composite of straight-alpha ARGB `src` onto `dst`.
fn blend_over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xff;
    if sa == 0 {
        return dst;
    }
    if sa == 255 {
        return src;
    }
    let da = (dst >> 24) & 0xff;
    let inv = 255 - sa;
    let out_a = sa + da * inv / 255;
    let ch = |s: u32, d: u32| -> u32 {
        let sc = s * sa;
        let dc = d * da * inv / 255;
        if out_a == 0 { 0 } else { ((sc + dc) / out_a).min(255) }
    };
    let r = ch((src >> 16) & 0xff, (dst >> 16) & 0xff);
    let g = ch((src >> 8) & 0xff, (dst >> 8) & 0xff);
    let b = ch(src & 0xff, dst & 0xff);
    (out_a << 24) | (r << 16) | (g << 8) | b
}

/// Flatten straight-alpha ARGB over opaque white -> XRGB.
fn over_white(p: u32) -> u32 {
    let a = (p >> 24) & 0xff;
    if a == 255 {
        return 0xff00_0000 | (p & 0x00ff_ffff);
    }
    if a == 0 {
        return 0xffff_ffff;
    }
    let inv = 255 - a;
    let ch = |c: u32| (c * a + 255 * inv) / 255;
    0xff00_0000
        | (ch((p >> 16) & 0xff) << 16)
        | (ch((p >> 8) & 0xff) << 8)
        | ch(p & 0xff)
}

/// Multiply a straight-alpha color's alpha by `ga` (globalAlpha).
fn with_alpha(c: u32, ga: f64) -> u32 {
    let a = ((c >> 24) & 0xff) as f64 * ga;
    ((round(a).clamp(0.0, 255.0) as u32) << 24) | (c & 0x00ff_ffff)
}

fn color_str(c: u32) -> String {
    alloc::format!("#{:06x}", c & 0xffffff)
}
fn num_str(n: f64) -> String {
    super::value::num_to_str(n)
}

/// Parse a CSS color into straight-alpha ARGB. Supports #rgb/#rrggbb,
/// rgb()/rgba(), hsl()/hsla(), and the common named colors.
pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    let low = v.to_ascii_lowercase();
    if let Some(inner) = low.strip_prefix("rgba(").or_else(|| low.strip_prefix("rgb(")) {
        let parts: Vec<&str> = inner.trim_end_matches(')').split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let r = parse_chan(parts[0]);
            let g = parse_chan(parts[1]);
            let b = parse_chan(parts[2]);
            let a = if parts.len() >= 4 {
                (parts[3].parse::<f64>().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0) as u32
            } else {
                255
            };
            return Some((a << 24) | (r << 16) | (g << 8) | b);
        }
    }
    if let Some(inner) = low.strip_prefix("hsla(").or_else(|| low.strip_prefix("hsl(")) {
        let parts: Vec<&str> = inner.trim_end_matches(')').split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let h = parts[0].trim_end_matches("deg").parse::<f64>().unwrap_or(0.0);
            let s = parts[1].trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0;
            let l = parts[2].trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0;
            let a = if parts.len() >= 4 {
                (parts[3].parse::<f64>().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0) as u32
            } else {
                255
            };
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some((a << 24) | (r << 16) | (g << 8) | b);
        }
    }
    if let Some(hex) = low.strip_prefix('#') {
        let h = hex.as_bytes();
        let d = |c: u8| (c as char).to_digit(16);
        return match h.len() {
            3 => Some(0xff00_0000 | d(h[0])? * 0x11 << 16 | d(h[1])? * 0x11 << 8 | d(h[2])? * 0x11),
            6 => Some(0xff00_0000 | (d(h[0])? << 4 | d(h[1])?) << 16 | (d(h[2])? << 4 | d(h[3])?) << 8 | (d(h[4])? << 4 | d(h[5])?)),
            8 => Some((d(h[6])? << 4 | d(h[7])?) << 24 | (d(h[0])? << 4 | d(h[1])?) << 16 | (d(h[2])? << 4 | d(h[3])?) << 8 | (d(h[4])? << 4 | d(h[5])?)),
            _ => None,
        };
    }
    Some(match low.as_str() {
        "black" => 0xff00_0000,
        "white" => 0xffff_ffff,
        "red" => 0xffff_0000,
        "green" => 0xff00_8000,
        "lime" => 0xff00_ff00,
        "blue" => 0xff00_00ff,
        "yellow" => 0xffff_ff00,
        "orange" => 0xffff_a500,
        "purple" => 0xff80_0080,
        "gray" | "grey" => 0xff80_8080,
        "lightgray" | "lightgrey" => 0xffd3_d3d3,
        "darkgray" | "darkgrey" => 0xffa9_a9a9,
        "cyan" | "aqua" => 0xff00_ffff,
        "magenta" | "fuchsia" => 0xffff_00ff,
        "pink" => 0xffff_c0cb,
        "brown" => 0xffa5_2a2a,
        "navy" => 0xff00_0080,
        "teal" => 0xff00_8080,
        "silver" => 0xffc0_c0c0,
        "gold" => 0xffff_d700,
        "transparent" => 0x0000_0000,
        _ => return None,
    })
}

fn parse_chan(s: &str) -> u32 {
    if let Some(p) = s.strip_suffix('%') {
        (p.trim().parse::<f64>().unwrap_or(0.0) / 100.0 * 255.0).clamp(0.0, 255.0) as u32
    } else {
        s.parse::<f64>().unwrap_or(0.0).clamp(0.0, 255.0) as u32
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u32, u32, u32) {
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    if s == 0.0 {
        let v = (l * 255.0) as u32;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f64| -> u32 {
        let mut t = t;
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).clamp(0.0, 255.0) as u32
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

// --- small math (kernel has no libm) -----------------------------------------

fn sin(x: f64) -> f64 { taylor_sin(reduce(x)) }
fn cos(x: f64) -> f64 { taylor_sin(reduce(x + core::f64::consts::FRAC_PI_2)) }

fn reduce(x: f64) -> f64 {
    let tau = 2.0 * core::f64::consts::PI;
    let mut x = x % tau;
    if x > core::f64::consts::PI { x -= tau; }
    if x < -core::f64::consts::PI { x += tau; }
    x
}
fn taylor_sin(x: f64) -> f64 {
    // 7th-order Taylor; |x| <= pi keeps it accurate enough for arc flattening.
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))))
}

fn inv_apply(t: &Affine, x: f64, y: f64) -> (f64, f64) {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-9 {
        return (x, y);
    }
    let id = 1.0 / det;
    let (px, py) = (x - t.e, y - t.f);
    (( t.d * px - t.c * py) * id, (-t.b * px + t.a * py) * id)
}
