//! Minimal PNG decoder for the browser (M16): a from-scratch zlib
//! inflate (stored, fixed and dynamic Huffman blocks, puff-style
//! bit-at-a-time canonical decode) plus PNG scanline unfiltering.
//!
//! Supported: 8-bit truecolor (color type 2) and truecolor+alpha (6),
//! non-interlaced. Alpha is dropped (no compositing translucency).
//! Output is XRGB8888 rows, ready to blit.

use alloc::vec;
use alloc::vec::Vec;

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<u32>,
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

fn inflate_codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff) -> Option<()> {
    loop {
        let sym = lit.decode(b)? as usize;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Some(()),
            257..=285 => {
                let len =
                    LBASE[sym - 257] as usize + b.bits(LEXT[sym - 257] as u32)? as usize;
                let d = dist.decode(b)? as usize;
                if d >= 30 {
                    return None;
                }
                let back = DBASE[d] as usize + b.bits(DEXT[d] as u32)? as usize;
                if back > out.len() {
                    return None;
                }
                for _ in 0..len {
                    out.push(out[out.len() - back]);
                }
            }
            _ => return None,
        }
    }
}

/// zlib stream -> raw bytes (adler32 trailer unchecked).
pub fn inflate(src: &[u8]) -> Option<Vec<u8>> {
    if src.len() < 2 || src[0] & 0x0f != 8 {
        return None;
    }
    let mut b = Bits { data: &src[2..], byte: 0, bit: 0 };
    let mut out = Vec::new();
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
                    out.push(b.bits(8)? as u8);
                }
            }
            1 => {
                let mut ll = [0u8; 288];
                ll[0..144].fill(8);
                ll[144..256].fill(9);
                ll[256..280].fill(7);
                ll[280..288].fill(8);
                inflate_codes(&mut b, &mut out, &build(&ll), &build(&[5u8; 30]))?;
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
                inflate_codes(
                    &mut b,
                    &mut out,
                    &build(&lens[..hlit]),
                    &build(&lens[hlit..]),
                )?;
            }
            _ => return None,
        }
        if last == 1 {
            return Some(out);
        }
    }
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

pub fn decode(data: &[u8]) -> Option<Image> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 8 || data[..8] != SIG {
        return None;
    }
    let be32 = |o: usize| -> Option<usize> {
        Some(u32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    let (mut w, mut h, mut bpp) = (0usize, 0usize, 0usize);
    let mut idat = Vec::new();
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
                // bit depth 8, color type 2/6, no interlace only
                if body[8] != 8 || body[12] != 0 {
                    return None;
                }
                bpp = match body[9] {
                    2 => 3,
                    6 => 4,
                    _ => return None,
                };
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len; // len + tag + body + crc
    }
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let raw = inflate(&idat)?;
    let stride = w * bpp;
    if raw.len() < h * (stride + 1) {
        return None;
    }
    let mut prev = vec![0u8; stride];
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let filt = row[0];
        let mut cur = row[1..].to_vec();
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let up = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = cur[i].wrapping_add(match filt {
                0 => 0,
                1 => a,
                2 => up,
                3 => ((a as u16 + up as u16) / 2) as u8,
                4 => paeth(a, up, c),
                _ => return None,
            });
        }
        for px in cur.chunks_exact(bpp) {
            pixels.push(
                0xff00_0000 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32,
            );
        }
        prev = cur;
    }
    Some(Image { w, h, pixels })
}
