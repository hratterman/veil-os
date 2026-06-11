//! M35: a from-scratch JPEG decoder — no crates. Handles **baseline** (SOF0/1)
//! *and* **progressive** (SOF2) DCT JPEGs: markers, quantisation tables,
//! Huffman DC/AC tables, restart markers, an integer separable IDCT, chroma
//! upsampling (4:4:4 / 4:2:2 / 4:2:0 and friends) and YCbCr→RGB. Progressive
//! support matters because most web JPEGs are progressive. Output is the same
//! `png::Image` (XRGB8888) so the viewer, file manager and browser blit it
//! unchanged. The entropy decode follows the canonical stb_image structure.

use crate::png::Image;
use alloc::vec;
use alloc::vec::Vec;

const SHIFT: i64 = 11;
const M: [[i32; 8]; 8] = [
    [724, 724, 724, 724, 724, 724, 724, 724],
    [1004, 851, 569, 200, -200, -569, -851, -1004],
    [946, 392, -392, -946, -946, -392, 392, 946],
    [851, -200, -1004, -569, 569, 1004, 200, -851],
    [724, -724, -724, 724, 724, -724, -724, 724],
    [569, -1004, 200, 851, -851, -200, 1004, -569],
    [392, -946, 946, -392, -392, 946, -946, 392],
    [200, -569, 851, -1004, 1004, -851, 569, -200],
];

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

struct Huff {
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [usize; 17],
    syms: Vec<u8>,
}

impl Huff {
    fn build(counts: &[u8; 16], syms: Vec<u8>) -> Huff {
        let mut mincode = [0i32; 17];
        let mut maxcode = [-1i32; 17];
        let mut valptr = [0usize; 17];
        let mut code = 0i32;
        let mut k = 0usize;
        for len in 1..=16 {
            let n = counts[len - 1] as i32;
            if n > 0 {
                valptr[len] = k;
                mincode[len] = code;
                code += n;
                maxcode[len] = code - 1;
                k += n as usize;
            }
            code <<= 1;
        }
        Huff { mincode, maxcode, valptr, syms }
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u32,
    nbits: u32,
    marker: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        BitReader { data, pos, bits: 0, nbits: 0, marker: 0 }
    }

    fn bit(&mut self) -> u32 {
        if self.nbits == 0 {
            if self.marker != 0 || self.pos >= self.data.len() {
                return 0;
            }
            let mut b = self.data[self.pos];
            self.pos += 1;
            if b == 0xFF {
                while self.pos < self.data.len() && self.data[self.pos] == 0xFF {
                    self.pos += 1;
                }
                if self.pos < self.data.len() {
                    let m = self.data[self.pos];
                    self.pos += 1;
                    if m != 0x00 {
                        self.marker = m;
                        b = 0;
                    }
                }
            }
            self.bits = b as u32;
            self.nbits = 8;
        }
        self.nbits -= 1;
        (self.bits >> self.nbits) & 1
    }

    fn receive(&mut self, n: u32) -> i32 {
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.bit() as i32;
        }
        v
    }

    fn decode(&mut self, h: &Huff) -> Option<u8> {
        let mut code = 0i32;
        for len in 1..=16 {
            code = (code << 1) | self.bit() as i32;
            if h.maxcode[len] >= 0 && code <= h.maxcode[len] {
                let idx = h.valptr[len] + (code - h.mincode[len]) as usize;
                return h.syms.get(idx).copied();
            }
        }
        None
    }

    /// At a restart boundary: drop partial bits and swallow the RSTn marker.
    fn restart(&mut self) {
        self.nbits = 0;
        if self.marker >= 0xD0 && self.marker <= 0xD7 {
            self.marker = 0;
            return;
        }
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            if self.data[self.pos] == 0xFF && self.data[self.pos + 1] != 0x00 {
                return;
            }
            self.pos += 1;
        }
    }
}

fn extend(v: i32, n: u32) -> i32 {
    if n == 0 {
        0
    } else if v < (1 << (n - 1)) {
        v - (1 << n) + 1
    } else {
        v
    }
}

struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    td: usize,
    ta: usize,
    pred: i32,
    bwb: usize, // coefficient blocks across (padded to MCU)
    bhb: usize,
    cx: usize, // component pixel width  (ceil(w*h/hmax))
    cy: usize,
    coef: Vec<i32>, // bwb*bhb*64 coefficients, natural 8x8 order
}

fn be16(d: &[u8], i: usize) -> usize {
    ((d[i] as usize) << 8) | d[i + 1] as usize
}

fn idct_block(coef: &[i32], qn: &[i32; 64], out: &mut [u8], stride: usize, ox: usize, oy: usize, w: usize, h: usize) {
    let mut blk = [0i32; 64];
    for i in 0..64 {
        blk[i] = coef[i] * qn[i];
    }
    let mut tmp = [0i64; 64];
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0i64;
            for u in 0..8 {
                s += M[u][y] as i64 * blk[u * 8 + x] as i64;
            }
            tmp[y * 8 + x] = s;
        }
    }
    for y in 0..8 {
        if oy + y >= h {
            break;
        }
        for x in 0..8 {
            if ox + x >= w {
                break;
            }
            let mut s = 0i64;
            for u in 0..8 {
                s += M[u][x] as i64 * tmp[y * 8 + u];
            }
            out[(oy + y) * stride + (ox + x)] = ((s >> (2 * SHIFT)) + 128).clamp(0, 255) as u8;
        }
    }
}

struct Scan {
    sel: Vec<usize>, // component indices in this scan, in order
    ss: usize,
    se: usize,
    ah: u32,
    al: u32,
    start: usize, // entropy data offset
}

pub fn decode(data: &[u8]) -> Option<Image> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut qn: [[i32; 64]; 4] = [[1; 64]; 4]; // natural-order quant tables
    let mut dc_huff: [Option<Huff>; 4] = [None, None, None, None];
    let mut ac_huff: [Option<Huff>; 4] = [None, None, None, None];
    let (mut w, mut h) = (0usize, 0usize);
    let mut comps: Vec<Comp> = Vec::new();
    let mut restart = 0usize;
    let mut progressive = false;
    let mut scan_count = 0usize;
    let mut hmax = 1;
    let mut vmax = 1;
    let (mut mcux, mut mcuy) = (0usize, 0usize);

    let mut p = 2;
    loop {
        if p + 1 >= data.len() {
            break;
        }
        if data[p] != 0xFF {
            p += 1;
            continue;
        }
        let marker = data[p + 1];
        p += 2;
        match marker {
            0xD9 => break, // EOI
            0xC0 | 0xC1 | 0xC2 => {
                progressive = marker == 0xC2;
                let len = be16(data, p);
                let seg = data.get(p + 2..p + len)?;
                h = be16(seg, 1);
                w = be16(seg, 3);
                let nc = seg[5] as usize;
                for i in 0..nc {
                    let o = 6 + i * 3;
                    comps.push(Comp {
                        id: seg[o],
                        h: (seg[o + 1] >> 4).max(1) as usize,
                        v: (seg[o + 1] & 0xF).max(1) as usize,
                        tq: seg[o + 2] as usize,
                        td: 0,
                        ta: 0,
                        pred: 0,
                        bwb: 0,
                        bhb: 0,
                        cx: 0,
                        cy: 0,
                        coef: Vec::new(),
                    });
                }
                if w == 0 || h == 0 || w > 6000 || h > 6000 {
                    return None;
                }
                hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
                vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
                mcux = w.div_ceil(8 * hmax);
                mcuy = h.div_ceil(8 * vmax);
                for c in comps.iter_mut() {
                    c.bwb = mcux * c.h;
                    c.bhb = mcuy * c.v;
                    c.cx = (w * c.h).div_ceil(hmax);
                    c.cy = (h * c.v).div_ceil(vmax);
                    c.coef = vec![0i32; c.bwb * c.bhb * 64];
                }
                p += len;
            }
            0xDB => {
                let len = be16(data, p);
                let mut o = p + 2;
                let end = p + len;
                while o < end {
                    let prec = data[o] >> 4;
                    let id = (data[o] & 0xF) as usize;
                    o += 1;
                    if id >= 4 {
                        return None;
                    }
                    for k in 0..64 {
                        let v = if prec == 0 {
                            let x = data[o] as i32;
                            o += 1;
                            x
                        } else {
                            let x = be16(data, o) as i32;
                            o += 2;
                            x
                        };
                        qn[id][ZIGZAG[k]] = v; // store in natural order
                    }
                }
                p += len;
            }
            0xC4 => {
                let len = be16(data, p);
                let mut o = p + 2;
                let end = p + len;
                while o < end {
                    let class = data[o] >> 4;
                    let id = (data[o] & 0xF) as usize;
                    o += 1;
                    let mut counts = [0u8; 16];
                    let mut total = 0usize;
                    for c in &mut counts {
                        *c = data[o];
                        total += data[o] as usize;
                        o += 1;
                    }
                    let syms = data.get(o..o + total)?.to_vec();
                    o += total;
                    if id >= 4 {
                        return None;
                    }
                    let hf = Huff::build(&counts, syms);
                    if class == 0 {
                        dc_huff[id] = Some(hf);
                    } else {
                        ac_huff[id] = Some(hf);
                    }
                }
                p += len;
            }
            0xDD => {
                restart = be16(data, p + 2);
                p += 4;
            }
            0xDA => {
                let len = be16(data, p);
                let seg = data.get(p + 2..p + len)?;
                let ns = seg[0] as usize;
                let mut sel = Vec::new();
                for i in 0..ns {
                    let cid = seg[1 + i * 2];
                    let t = seg[2 + i * 2];
                    if let Some(idx) = comps.iter().position(|c| c.id == cid) {
                        comps[idx].td = (t >> 4) as usize;
                        comps[idx].ta = (t & 0xF) as usize;
                        sel.push(idx);
                    }
                }
                let ss = seg[1 + ns * 2] as usize;
                let se = seg[2 + ns * 2] as usize;
                let aa = seg[3 + ns * 2];
                let scan = Scan { sel, ss, se, ah: (aa >> 4) as u32, al: (aa & 0xF) as u32, start: p + len };
                // Decode this scan's entropy data now (we need the live huffman
                // tables, which can change between scans).
                decode_scan(data, &scan, &mut comps, mcux, mcuy, restart, progressive, &dc_huff, &ac_huff)?;
                scan_count += 1;
                // Skip past entropy data to the next marker.
                p = scan.start;
                while p + 1 < data.len() && !(data[p] == 0xFF && data[p + 1] != 0x00 && !(0xD0..=0xD7).contains(&data[p + 1])) {
                    p += 1;
                }
            }
            0x01 => {}
            0xD0..=0xD7 => {}
            _ => {
                let len = be16(data, p);
                p += len;
            }
        }
    }

    if comps.is_empty() || scan_count == 0 {
        return None;
    }

    // Finalise: dequantise + IDCT each component to its native pixel plane,
    // then upsample + colour-convert.
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(comps.len());
    for c in &comps {
        let stride = c.bwb * 8;
        let mut plane = vec![0u8; stride * c.bhb * 8];
        let q = &qn[c.tq.min(3)];
        for by in 0..c.bhb {
            for bx in 0..c.bwb {
                let off = (by * c.bwb + bx) * 64;
                idct_block(&c.coef[off..off + 64], q, &mut plane, stride, bx * 8, by * 8, stride, c.bhb * 8);
            }
        }
        planes.push(plane);
    }

    let mut pixels = vec![0u32; w * h];
    let nc = comps.len();
    for y in 0..h {
        for x in 0..w {
            let sample = |ci: usize| -> i32 {
                let c = &comps[ci];
                let stride = c.bwb * 8;
                let sx = (x * c.h / hmax).min(c.cx - 1);
                let sy = (y * c.v / vmax).min(c.cy - 1);
                planes[ci][sy * stride + sx] as i32
            };
            let rgb = if nc == 1 {
                let g = sample(0).clamp(0, 255) as u32;
                0xff00_0000 | g << 16 | g << 8 | g
            } else {
                let yy = sample(0);
                let cb = sample(1) - 128;
                let cr = sample(2) - 128;
                let r = (yy + ((91881 * cr) >> 16)).clamp(0, 255) as u32;
                let g = (yy - ((22554 * cb + 46802 * cr) >> 16)).clamp(0, 255) as u32;
                let b = (yy + ((116130 * cb) >> 16)).clamp(0, 255) as u32;
                0xff00_0000 | r << 16 | g << 8 | b
            };
            pixels[y * w + x] = rgb;
        }
    }
    Some(Image { w, h, full_w: w, full_h: h, pixels })
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    data: &[u8],
    scan: &Scan,
    comps: &mut [Comp],
    mcux: usize,
    mcuy: usize,
    restart: usize,
    progressive: bool,
    dc_huff: &[Option<Huff>; 4],
    ac_huff: &[Option<Huff>; 4],
) -> Option<()> {
    let mut br = BitReader::new(data, scan.start);
    for c in comps.iter_mut() {
        c.pred = 0;
    }
    let mut eob_run = 0i32;
    let interleaved = scan.sel.len() > 1;

    if interleaved {
        // Baseline or progressive DC interleaved scan: walk MCUs.
        let mut count = 0usize;
        for my in 0..mcuy {
            for mx in 0..mcux {
                if restart != 0 && count != 0 && count % restart == 0 {
                    br.restart();
                    for c in comps.iter_mut() {
                        c.pred = 0;
                    }
                    eob_run = 0;
                }
                for &ci in &scan.sel {
                    let (ch, cv, td, ta, bwb) =
                        (comps[ci].h, comps[ci].v, comps[ci].td, comps[ci].ta, comps[ci].bwb);
                    for by in 0..cv {
                        for bx in 0..ch {
                            let bxx = mx * ch + bx;
                            let byy = my * cv + by;
                            let off = (byy * bwb + bxx) * 64;
                            decode_block(
                                &mut br, &mut comps[ci].pred, off, &mut comps[ci].coef, scan,
                                progressive, &mut eob_run, dc_huff.get(td)?.as_ref()?,
                                ac_huff.get(ta)?.as_ref(),
                            )?;
                        }
                    }
                }
                count += 1;
            }
        }
    } else {
        // Non-interleaved scan (progressive AC, or single-component): walk the
        // component's own block grid.
        let ci = scan.sel[0];
        let (td, ta, bwb) = (comps[ci].td, comps[ci].ta, comps[ci].bwb);
        let nbx = comps[ci].cx.div_ceil(8);
        let nby = comps[ci].cy.div_ceil(8);
        let mut count = 0usize;
        for by in 0..nby {
            for bx in 0..nbx {
                if restart != 0 && count != 0 && count % restart == 0 {
                    br.restart();
                    comps[ci].pred = 0;
                    eob_run = 0;
                }
                let off = (by * bwb + bx) * 64;
                decode_block(
                    &mut br, &mut comps[ci].pred, off, &mut comps[ci].coef, scan, progressive,
                    &mut eob_run, dc_huff.get(td)?.as_ref()?, ac_huff.get(ta)?.as_ref(),
                )?;
                count += 1;
            }
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn decode_block(
    br: &mut BitReader,
    pred: &mut i32,
    off: usize,
    coef: &mut [i32],
    scan: &Scan,
    progressive: bool,
    eob_run: &mut i32,
    dc: &Huff,
    ac: Option<&Huff>,
) -> Option<()> {
    let blk = &mut coef[off..off + 64];
    if !progressive {
        // Baseline: full block.
        let t = br.decode(dc)? as u32;
        let diff = extend(br.receive(t), t);
        *pred += diff;
        blk[0] = *pred;
        let ac = ac?;
        let mut k = 1usize;
        while k < 64 {
            let rs = br.decode(ac)?;
            let r = (rs >> 4) as usize;
            let s = (rs & 0xF) as u32;
            if s == 0 {
                if r != 15 {
                    break;
                }
                k += 16;
                continue;
            }
            k += r;
            if k >= 64 {
                break;
            }
            blk[ZIGZAG[k]] = extend(br.receive(s), s);
            k += 1;
        }
        return Some(());
    }

    // Progressive.
    if scan.ss == 0 {
        // DC scan.
        if scan.ah == 0 {
            let t = br.decode(dc)? as u32;
            let diff = extend(br.receive(t), t);
            *pred += diff;
            blk[0] = *pred << scan.al;
        } else if br.bit() != 0 {
            blk[0] |= 1 << scan.al;
        }
        return Some(());
    }

    // AC scan (single component).
    let ac = ac?;
    let (ss, se) = (scan.ss, scan.se);
    if scan.ah == 0 {
        // First AC scan in this band.
        if *eob_run > 0 {
            *eob_run -= 1;
            return Some(());
        }
        let mut k = ss;
        while k <= se {
            let rs = br.decode(ac)?;
            let r = (rs >> 4) as usize;
            let s = (rs & 0xF) as u32;
            if s == 0 {
                if r < 15 {
                    *eob_run = (1 << r) - 1;
                    if r > 0 {
                        *eob_run += br.receive(r as u32);
                    }
                    break;
                }
                k += 16;
            } else {
                k += r;
                if k > se {
                    break;
                }
                blk[ZIGZAG[k]] = extend(br.receive(s), s) << scan.al;
                k += 1;
            }
        }
    } else {
        // AC refinement scan.
        let bit = 1i32 << scan.al;
        let mut k = ss;
        if *eob_run == 0 {
            while k <= se {
                let rs = br.decode(ac)?;
                let mut r = (rs >> 4) as i32;
                let s = (rs & 0xF) as u32;
                let mut val = 0i32;
                if s == 0 {
                    if r < 15 {
                        *eob_run = (1 << r) - 1;
                        if r > 0 {
                            *eob_run += br.receive(r as u32);
                        }
                        break;
                    }
                    // r == 15: skip 16 zero-history coefficients.
                } else {
                    // s must be 1: a newly-nonzero coefficient.
                    val = if br.bit() != 0 { bit } else { -bit };
                }
                // Advance, applying correction bits to existing nonzeros until we
                // have passed `r` zero-history coefficients, then place `val`.
                while k <= se {
                    let z = ZIGZAG[k];
                    if blk[z] != 0 {
                        if br.bit() != 0 && (blk[z] & bit) == 0 {
                            blk[z] += if blk[z] > 0 { bit } else { -bit };
                        }
                    } else {
                        if r == 0 {
                            if val != 0 {
                                blk[z] = val;
                            }
                            k += 1;
                            break;
                        }
                        r -= 1;
                    }
                    k += 1;
                }
            }
        }
        if *eob_run > 0 {
            while k <= se {
                let z = ZIGZAG[k];
                if blk[z] != 0 && br.bit() != 0 && (blk[z] & bit) == 0 {
                    blk[z] += if blk[z] > 0 { bit } else { -bit };
                }
                k += 1;
            }
            *eob_run -= 1;
        }
    }
    Some(())
}
