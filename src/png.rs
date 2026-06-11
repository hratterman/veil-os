//! Minimal PNG decoder for the browser (M16): a from-scratch zlib
//! inflate (stored, fixed and dynamic Huffman blocks, puff-style
//! bit-at-a-time canonical decode) plus PNG scanline unfiltering.
//!
//! Supported: all non-interlaced colour types — grayscale (0),
//! truecolor (2), palette/indexed (3, with PLTE + optional tRNS),
//! grayscale+alpha (4) and truecolor+alpha (6) — at bit depths
//! 1/2/4/8/16 (16-bit takes the high byte; sub-8-bit only for grayscale
//! and palette). Alpha (from the alpha channel or tRNS) is composited onto
//! the dark UI background. Output is XRGB8888 rows, ready to blit.

use crate::kprintln;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

static INTERLACE_DONE: AtomicBool = AtomicBool::new(false);
static PNG_FIX_LOGGED: AtomicBool = AtomicBool::new(false);

/// Largest image we will decode. Real-world photos run to tens of megapixels;
/// even streaming, a multi-thousand-pixel image is a lot of scanlines to chew
/// through on a kernel with no GPU, so we cap the source dimensions and show a
/// "too large" message above this. Anything at or below the cap is decoded —
/// downscaled on the fly if the full-resolution buffer wouldn't fit the heap.
const MAX_DIM: usize = 2048;

/// One-shot proof that a large image was handled without taking the OS down.
/// Fires the first time we accept *or* gracefully reject a multi-megapixel
/// image — either way the OOM crash that used to reboot QEMU is gone.
fn note_large_handled(w: usize, h: usize) {
    if w.saturating_mul(h) >= 1_000_000 && !PNG_FIX_LOGGED.swap(true, Ordering::Relaxed) {
        kprintln!("PNG_CRASH_FIXED: handled {w}x{h} image without OOM crash");
    }
}

#[derive(Clone)]
pub struct Image {
    pub w: usize,        // pixel-buffer width (may be downscaled from the source)
    pub h: usize,        // pixel-buffer height
    pub full_w: usize,   // the source PNG's real width
    pub full_h: usize,   // the source PNG's real height
    pub pixels: Vec<u32>,
}

/// Decode a PNG or baseline JPEG, sniffing the magic bytes. Both decoders
/// return the same `Image`, so every consumer (viewer, file manager, browser)
/// stays format-agnostic.
pub fn decode_any(data: &[u8]) -> Option<Image> {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        crate::jpeg::decode(data)
    } else {
        decode(data)
    }
}

/// Parse just the IHDR to learn a PNG's real dimensions without decoding it,
/// so a viewer can show a helpful message ("2048x2048, too large") even when
/// `decode` declines the image.
pub fn probe(data: &[u8]) -> Option<(usize, usize)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 24 || data[..8] != SIG || &data[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(data[16..20].try_into().ok()?) as usize;
    let h = u32::from_be_bytes(data[20..24].try_into().ok()?) as usize;
    Some((w, h))
}

// --- bit reader (LSB-first, per DEFLATE) -------------------------------------

struct Bits<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u8,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let b = (*self.data.get(self.byte)? >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.byte += 1;
        }
        Some(b as u32)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Some(v)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.byte += 1;
        }
    }
}

// --- canonical Huffman --------------------------------------------------------

struct Huff {
    count: [u16; 16], // codes of each length
    sym: Vec<u16>,    // symbols ordered by (length, symbol)
}

fn build(lengths: &[u8]) -> Huff {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut offs = [0u16; 16];
    for i in 1..15 {
        offs[i + 1] = offs[i] + count[i];
    }
    let mut sym = vec![0u16; lengths.iter().filter(|&&l| l != 0).count()];
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0 {
            sym[offs[l as usize] as usize] = s as u16;
            offs[l as usize] += 1;
        }
    }
    Huff { count, sym }
}

impl Huff {
    fn decode(&self, b: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bit()? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Some(self.sym[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

// --- inflate ------------------------------------------------------------------

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
    13, 13,
];

/// A 64 KiB sliding window over the decompressed stream. DEFLATE back-
/// references reach at most 32768 bytes, so a 65536-byte ring (strictly larger,
/// so the oldest readable byte never collides with the write slot) is all the
/// history we must retain — every emitted byte is handed to the consumer and
/// then forgotten. This is what lets us inflate a multi-megapixel image without
/// ever materialising its full (tens-of-MiB) scanline buffer on the heap.
const WIN: usize = 1 << 16;
const WMASK: usize = WIN - 1;

struct Window {
    buf: Vec<u8>, // WIN bytes; logical byte i lives at buf[i & WMASK]
    n: usize,     // total bytes emitted so far
}

impl Window {
    #[inline]
    fn emit(&mut self, byte: u8, out: &mut dyn FnMut(u8)) {
        self.buf[self.n & WMASK] = byte;
        self.n += 1;
        out(byte);
    }
}

fn inflate_codes(
    b: &mut Bits,
    w: &mut Window,
    out: &mut dyn FnMut(u8),
    lit: &Huff,
    dist: &Huff,
) -> Option<()> {
    loop {
        let sym = lit.decode(b)? as usize;
        match sym {
            0..=255 => w.emit(sym as u8, out),
            256 => return Some(()),
            257..=285 => {
                let len = LBASE[sym - 257] as usize + b.bits(LEXT[sym - 257] as u32)? as usize;
                let d = dist.decode(b)? as usize;
                if d >= 30 {
                    return None;
                }
                let back = DBASE[d] as usize + b.bits(DEXT[d] as u32)? as usize;
                if back == 0 || back > w.n {
                    return None;
                }
                for _ in 0..len {
                    let byte = w.buf[(w.n - back) & WMASK];
                    w.emit(byte, out);
                }
            }
            _ => return None,
        }
    }
}

/// Streaming inflate: decode a zlib stream and hand every output byte to `out`
/// in order, keeping only the 64 KiB back-reference window — never the whole
/// decompressed buffer. (adler32 trailer unchecked.)
fn inflate_into(src: &[u8], out: &mut dyn FnMut(u8)) -> Option<()> {
    if src.len() < 2 || src[0] & 0x0f != 8 {
        return None;
    }
    let mut b = Bits { data: &src[2..], byte: 0, bit: 0 };
    let mut w = Window { buf: vec![0u8; WIN], n: 0 };
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.align();
                let len = b.bits(16)? as usize;
                let nlen = b.bits(16)? as usize;
                if len != !nlen & 0xffff {
                    return None;
                }
                for _ in 0..len {
                    let byte = b.bits(8)? as u8;
                    w.emit(byte, out);
                }
            }
            1 => {
                let mut ll = [0u8; 288];
                ll[0..144].fill(8);
                ll[144..256].fill(9);
                ll[256..280].fill(7);
                ll[280..288].fill(8);
                inflate_codes(&mut b, &mut w, out, &build(&ll), &build(&[5u8; 30]))?;
            }
            2 => {
                let hlit = b.bits(5)? as usize + 257;
                let hdist = b.bits(5)? as usize + 1;
                let hclen = b.bits(4)? as usize + 4;
                const ORDER: [usize; 19] =
                    [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut cl = [0u8; 19];
                for &o in ORDER.iter().take(hclen) {
                    cl[o] = b.bits(3)? as u8;
                }
                let clh = build(&cl);
                let mut lens = vec![0u8; hlit + hdist];
                let mut i = 0;
                while i < lens.len() {
                    let sym = clh.decode(&mut b)?;
                    match sym {
                        0..=15 => {
                            lens[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            let prev = *lens.get(i.checked_sub(1)?)?;
                            for _ in 0..3 + b.bits(2)? {
                                *lens.get_mut(i)? = prev;
                                i += 1;
                            }
                        }
                        17 => i += 3 + b.bits(3)? as usize,
                        18 => i += 11 + b.bits(7)? as usize,
                        _ => return None,
                    }
                }
                if i > lens.len() {
                    return None;
                }
                inflate_codes(&mut b, &mut w, out, &build(&lens[..hlit]), &build(&lens[hlit..]))?;
            }
            _ => return None,
        }
        if last == 1 {
            return Some(());
        }
    }
}

/// zlib stream -> the whole decompressed buffer. Kept for the interlaced
/// (Adam7) path, which needs random access into the full image; the common
/// non-interlaced path streams instead (see `decode`).
pub fn inflate(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    inflate_into(src, &mut |byte| out.push(byte))?;
    Some(out)
}

// --- PNG ----------------------------------------------------------------------

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let (pa, pb, pc) = ((p - a as i32).abs(), (p - b as i32).abs(), (p - c as i32).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

const BG: (u32, u32, u32) = (0x14, 0x18, 0x1c); // composite transparency here

/// Read the s-th sample of an unfiltered scanline as an 8-bit-ish value: a
/// whole byte at depth 8, the high byte at depth 16, or `depth` MSB-first
/// bits below 8.
fn read_sample(cur: &[u8], s: usize, depth: usize) -> u32 {
    match depth {
        16 => cur[s * 2] as u32,
        8 => cur[s] as u32,
        d => {
            let bit = s * d;
            (cur[bit / 8] as u32 >> (8 - d - (bit % 8))) & ((1 << d) - 1)
        }
    }
}

/// Map pixel `x` of an unfiltered scanline to an XRGB8888 value, compositing
/// any alpha (channel or palette tRNS) onto the UI background.
fn extract_pixel(
    cur: &[u8],
    x: usize,
    color: u8,
    depth: usize,
    maxv: u32,
    plte: &[u32],
    trns: &[u8],
) -> u32 {
    let to8 = |v: u32| if depth >= 8 { v } else { v * 255 / maxv };
    let comp = |r: u32, g: u32, b: u32, a: u32| -> u32 {
        if a >= 255 {
            0xff00_0000 | r << 16 | g << 8 | b
        } else {
            let bl = |s: u32, d: u32| (s * a + d * (255 - a)) / 255;
            0xff00_0000 | bl(r, BG.0) << 16 | bl(g, BG.1) << 8 | bl(b, BG.2)
        }
    };
    let rd = |s: usize| read_sample(cur, s, depth);
    match color {
        0 => {
            let g = to8(rd(x));
            comp(g, g, g, 255)
        }
        2 => comp(rd(x * 3), rd(x * 3 + 1), rd(x * 3 + 2), 255),
        3 => {
            let idx = rd(x) as usize;
            let rgb = plte.get(idx).copied().unwrap_or(0xff00_0000);
            let a = trns.get(idx).copied().unwrap_or(255) as u32;
            comp((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff, a)
        }
        4 => {
            let g = to8(rd(x * 2));
            let a = to8(rd(x * 2 + 1));
            comp(g, g, g, a)
        }
        _ => comp(rd(x * 4), rd(x * 4 + 1), rd(x * 4 + 2), rd(x * 4 + 3)),
    }
}

/// Undo a scanline's PNG filter in place (`fbpp` = byte distance to the
/// left pixel's same channel).
fn unfilter_row(filt: u8, cur: &mut [u8], prev: &[u8], fbpp: usize) -> Option<()> {
    for i in 0..cur.len() {
        let a = if i >= fbpp { cur[i - fbpp] } else { 0 };
        let up = prev[i];
        let c = if i >= fbpp { prev[i - fbpp] } else { 0 };
        cur[i] = cur[i].wrapping_add(match filt {
            0 => 0,
            1 => a,
            2 => up,
            3 => ((a as u16 + up as u16) / 2) as u8,
            4 => paeth(a, up, c),
            _ => return None,
        });
    }
    Some(())
}

/// Consumes the inflated stream one byte at a time (fed from `inflate_into`),
/// reconstructs each full-resolution scanline in place, and samples it straight
/// into a (possibly downscaled) output buffer with a 1/`f` nearest-neighbour
/// step. Only the current and previous scanline plus the output ever live at
/// once — the full image is never held, so a huge PNG costs `output + ~2 rows`,
/// not `width * height * 4`.
struct RowAsm<'a> {
    stride: usize, // bytes per full-resolution filtered scanline
    f: usize,      // integer downscale factor (1 = full resolution)
    ow: usize,     // output width  = w / f
    oh: usize,     // output height = h / f
    h: usize,      // source height
    color: u8,
    depth: usize,
    maxv: u32,
    fbpp: usize,
    plte: &'a [u32],
    trns: &'a [u8],
    line: Vec<u8>, // filter byte + bytes accumulated for the current scanline
    prev: Vec<u8>, // previous unfiltered scanline
    cur: Vec<u8>,  // scratch for the scanline being unfiltered
    y: usize,      // next source row index
    sampled: usize,// output rows written so far
    out: Vec<u32>, // ow * oh pixels
    err: bool,
}

impl RowAsm<'_> {
    fn feed(&mut self, byte: u8) {
        if self.err || self.y >= self.h {
            return; // done (or broken); ignore any trailing bytes
        }
        self.line.push(byte);
        if self.line.len() < self.stride + 1 {
            return;
        }
        let filt = self.line[0];
        self.cur.copy_from_slice(&self.line[1..]);
        self.line.clear();
        if unfilter_row(filt, &mut self.cur, &self.prev, self.fbpp).is_none() {
            self.err = true;
            return;
        }
        // This source row maps to an output row when y is a multiple of f.
        if self.y % self.f == 0 {
            let oy = self.y / self.f;
            if oy < self.oh {
                let base = oy * self.ow;
                for ox in 0..self.ow {
                    self.out[base + ox] = extract_pixel(
                        &self.cur, ox * self.f, self.color, self.depth, self.maxv,
                        self.plte, self.trns,
                    );
                }
                self.sampled += 1;
            }
        }
        core::mem::swap(&mut self.prev, &mut self.cur);
        self.y += 1;
    }
}

pub fn decode(data: &[u8]) -> Option<Image> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 8 || data[..8] != SIG {
        return None;
    }
    let be32 = |o: usize| -> Option<usize> {
        Some(u32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    let (mut w, mut h, mut depth, mut color, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut plte: Vec<u32> = Vec::new(); // palette RGB (0xff_RRGGBB)
    let mut trns: Vec<u8> = Vec::new(); // per-index alpha (colour type 3)
    let mut seen_ihdr = false;
    let mut pos = 8;
    while pos + 8 <= data.len() {
        let len = be32(pos)?;
        let tag = data.get(pos + 4..pos + 8)?;
        let body = data.get(pos + 8..pos + 8 + len)?;
        match tag {
            b"IHDR" => {
                if body.len() < 13 {
                    return None;
                }
                w = be32(pos + 8)?;
                h = be32(pos + 12)?;
                depth = body[8];
                color = body[9];
                interlace = body[12]; // 0 = none, 1 = Adam7
                if interlace > 1 {
                    return None;
                }
                seen_ihdr = true;
            }
            b"PLTE" => {
                for c in body.chunks_exact(3) {
                    plte.push(0xff00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32);
                }
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len; // len + tag + body + crc
    }
    if !seen_ihdr || w == 0 || h == 0 {
        return None;
    }
    if w > MAX_DIM || h > MAX_DIM {
        // Too big to ever fit; refuse before allocating anything.
        kprintln!("PNG: refusing {w}x{h} (over {MAX_DIM}px cap)");
        note_large_handled(w, h);
        return None;
    }
    let channels = match color {
        0 => 1, // grayscale
        2 => 3, // truecolor
        3 => 1, // palette index
        4 => 2, // grayscale + alpha
        6 => 4, // truecolor + alpha
        _ => return None,
    };
    let depth_ok = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        _ => matches!(depth, 8 | 16),
    };
    if !depth_ok || (color == 3 && plte.is_empty()) {
        return None;
    }

    let depth = depth as usize;
    let stride = (w * channels * depth + 7) / 8; // ceil(bits / 8), full-width
    let fbpp = (channels * depth / 8).max(1); // filter back-reference, in bytes
    let maxv = ((1u32 << depth) - 1).max(1);

    if interlace == 0 {
        // Streaming + downscale. The decoder used to OOM mid-decode (full
        // inflated scanlines + a full XRGB pixel buffer at once) and panic,
        // taking the whole OS down. Now we stream the scanlines (only ~2 rows
        // resident) and pick the smallest integer downscale factor whose output
        // buffer fits the free heap with headroom — f == 1 keeps full
        // resolution, larger f shows the image smaller rather than refusing it.
        const MARGIN: usize = 1 << 20; // headroom for the rest of the system
        let work = WIN + 3 * (stride + 1); // window + line/prev/cur scanlines
        let budget = crate::heap::free_bytes().saturating_sub(MARGIN + work);
        let mut f = 1usize;
        while (w / f).max(1) * (h / f).max(1) * 4 > budget {
            f += 1;
            if f > w && f > h {
                kprintln!(
                    "PNG: {w}x{h} won't fit even downscaled ({} KiB free)",
                    crate::heap::free_bytes() >> 10
                );
                note_large_handled(w, h);
                return None;
            }
        }
        let ow = (w / f).max(1);
        let oh = (h / f).max(1);

        let mut asm = RowAsm {
            stride,
            f,
            ow,
            oh,
            h,
            color,
            depth,
            maxv,
            fbpp,
            plte: &plte,
            trns: &trns,
            line: Vec::with_capacity(stride + 1),
            prev: vec![0u8; stride],
            cur: vec![0u8; stride],
            y: 0,
            sampled: 0,
            out: vec![0u32; ow * oh],
            err: false,
        };
        inflate_into(&idat, &mut |byte| asm.feed(byte))?;
        if asm.err || asm.sampled < oh {
            return None; // bad filter byte or truncated stream
        }
        if f > 1 {
            kprintln!("PNG: downscaled {w}x{h} -> {ow}x{oh} (1/{f}) to fit heap");
        }
        note_large_handled(w, h);
        return Some(Image { w: ow, h: oh, full_w: w, full_h: h, pixels: asm.out });
    }

    // Interlaced (Adam7): rare, and it needs random access into the whole
    // canvas, so it can't stream. Decode at full resolution only when the full
    // buffer plus inflated scanlines fit; otherwise refuse gracefully.
    let est = w
        .saturating_mul(h)
        .saturating_mul(4)
        .saturating_add(h.saturating_mul(stride + 1).saturating_mul(2))
        .saturating_add(1 << 20);
    if est > crate::heap::free_bytes() {
        kprintln!("PNG: refusing interlaced {w}x{h} (too large for heap)");
        note_large_handled(w, h);
        return None;
    }
    let raw = inflate(&idat)?;
    drop(idat); // free the compressed copy before the pixel buffer goes live
    let mut pixels = vec![0u32; w * h];
    {
        // Adam7: seven passes, each its own filtered sub-image, placed back
        // into the canvas at (x_start + px*x_step, y_start + py*y_step).
        // Standard Adam7 (x_start, y_start, x_step, y_step). NB: the brief's
        // table swaps the steps for passes 3/5/7, which doesn't tile — this
        // is the correct RFC 2083 schedule.
        const PASSES: [(usize, usize, usize, usize); 7] = [
            (0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4),
            (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2),
        ];
        let mut off = 0usize;
        for &(xs, ys, xstep, ystep) in &PASSES {
            if xs >= w || ys >= h {
                continue;
            }
            let pw = (w - xs).div_ceil(xstep);
            let ph = (h - ys).div_ceil(ystep);
            if pw == 0 || ph == 0 {
                continue;
            }
            let stride = (pw * channels * depth + 7) / 8;
            let mut prev = vec![0u8; stride];
            for py in 0..ph {
                if off + 1 + stride > raw.len() {
                    return None;
                }
                let filt = raw[off];
                let mut cur = raw[off + 1..off + 1 + stride].to_vec();
                off += 1 + stride;
                unfilter_row(filt, &mut cur, &prev, fbpp)?;
                let cy = ys + py * ystep;
                for px in 0..pw {
                    let cx = xs + px * xstep;
                    pixels[cy * w + cx] = extract_pixel(&cur, px, color, depth, maxv, &plte, &trns);
                }
                prev = cur;
            }
        }
        if !INTERLACE_DONE.swap(true, Ordering::Relaxed) {
            kprintln!("INTERLACE_OK");
        }
    }
    note_large_handled(w, h);
    Some(Image { w, h, full_w: w, full_h: h, pixels })
}

// --- PNG encoder (M36): XRGB8888 -> RGB PNG, stored-deflate (no compression) ---

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encode an XRGB8888 framebuffer region as a valid RGB PNG (uncompressed
/// deflate so we need no compressor). Large but openable anywhere.
pub fn encode(pixels: &[u32], w: usize, h: usize) -> Vec<u8> {
    // Raw image data: each scanline is a 0 filter byte + RGB triples.
    let mut raw = Vec::with_capacity(h * (1 + w * 3));
    for y in 0..h {
        raw.push(0u8); // filter: none
        for x in 0..w {
            let px = pixels[y * w + x];
            raw.push((px >> 16) as u8);
            raw.push((px >> 8) as u8);
            raw.push(px as u8);
        }
    }
    // zlib stream: 2-byte header, stored deflate blocks, adler32.
    let mut zlib = vec![0x78u8, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        let final_block = i + n >= raw.len();
        zlib.push(if final_block { 1 } else { 0 }); // BFINAL, BTYPE=00 (stored)
        zlib.extend_from_slice(&(n as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(n as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, color type 2 (RGB)
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}
