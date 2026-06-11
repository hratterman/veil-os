//! M31: a from-scratch GIF87a/GIF89a decoder — header, global/local colour
//! tables, LZW decompression, multi-frame animation (Graphic Control
//! Extension delay + disposal), interlacing, and transparent-index
//! compositing. Output is a `Gif` of pre-composited ARGB frames, so the
//! player just blits each one. No external crates.

use crate::kprintln;
use alloc::vec;
use alloc::vec::Vec;

pub struct GifFrame {
    pub delay_cs: u16,    // centiseconds; 0 -> caller uses a default
    pub pixels: Vec<u32>, // ARGB, canvas_w * canvas_h, pre-composited
}

pub struct Gif {
    pub w: u16,
    pub h: u16,
    pub frames: Vec<GifFrame>,
}

// Bounds so a hostile/huge upload can't exhaust the 16 MiB heap. We keep
// the canvas + a handful of frame snapshots; cap total snapshot memory.
const MAX_DIM: usize = 1024;
const MAX_FRAMES: usize = 256;
// Cap total decoded-frame memory: each frame keeps a full-canvas ARGB
// snapshot, and the heap is only 16 MiB shared with the rest of the OS.
// A big upload simply plays its first frames rather than crashing.
const MAX_PIXMEM: usize = 6 * 1024 * 1024 / 4; // ~6 MiB of ARGB pixels

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
    fn u16(&mut self) -> Option<u16> {
        let lo = self.u8()? as u16;
        let hi = self.u8()? as u16;
        Some(lo | hi << 8)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
    /// Read GIF data sub-blocks (a chain of [len][bytes]... ending in [0])
    /// into one contiguous buffer.
    fn sub_blocks(&mut self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let n = self.u8()? as usize;
            if n == 0 {
                return Some(out);
            }
            out.extend_from_slice(self.take(n)?);
        }
    }
    /// Skip a sub-block chain without keeping it (unknown extensions).
    fn skip_sub_blocks(&mut self) -> Option<()> {
        loop {
            let n = self.u8()? as usize;
            if n == 0 {
                return Some(());
            }
            self.take(n)?;
        }
    }
}

fn read_color_table(r: &mut Reader, entries: usize) -> Option<Vec<u32>> {
    let raw = r.take(entries * 3)?;
    let mut t = Vec::with_capacity(entries);
    for c in raw.chunks_exact(3) {
        t.push(0xff00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32);
    }
    Some(t)
}

/// LZW-decompress GIF image data into `out_len` palette indices.
fn lzw_decode(data: &[u8], min_code_size: u8, out_len: usize) -> Option<Vec<u8>> {
    if min_code_size < 2 || min_code_size > 8 {
        return None;
    }
    let clear = 1u16 << min_code_size;
    let eoi = clear + 1;
    // Code -> (prefix code, suffix byte); literals are their own byte.
    let mut prefix = [0u16; 4096];
    let mut suffix = [0u8; 4096];
    let mut stack = [0u8; 4096];
    let mut out = Vec::with_capacity(out_len);

    let mut code_size = min_code_size as u32 + 1;
    let mut avail = clear + 2;
    let mut oldcode: i32 = -1;
    let mut first: u8 = 0;

    let mut bitpos = 0usize;
    let total_bits = data.len() * 8;
    let mut read_code = |bitpos: &mut usize, cs: u32| -> Option<u16> {
        if *bitpos + cs as usize > total_bits {
            return None;
        }
        let mut code = 0u32;
        for k in 0..cs {
            let bp = *bitpos + k as usize;
            let bit = (data[bp >> 3] >> (bp & 7)) & 1;
            code |= (bit as u32) << k;
        }
        *bitpos += cs as usize;
        Some(code as u16)
    };

    loop {
        let Some(code) = read_code(&mut bitpos, code_size) else { break };
        if code == eoi {
            break;
        }
        if code == clear {
            code_size = min_code_size as u32 + 1;
            avail = clear + 2;
            oldcode = -1;
            continue;
        }
        if oldcode == -1 {
            // First code after a clear must be a literal.
            if code >= clear {
                return None;
            }
            out.push(code as u8);
            first = code as u8;
            oldcode = code as i32;
            continue;
        }
        // Walk the prefix chain into `stack`, then emit reversed.
        let mut sp = 0usize;
        let mut c = code;
        if c >= avail {
            // KwKwK: code not yet in the table.
            if c > avail {
                return None;
            }
            stack[sp] = first;
            sp += 1;
            c = oldcode as u16;
        }
        let mut guard = 0;
        while c >= clear {
            if sp >= stack.len() {
                return None;
            }
            stack[sp] = suffix[c as usize];
            sp += 1;
            c = prefix[c as usize];
            guard += 1;
            if guard > 4096 {
                return None;
            }
        }
        first = c as u8;
        stack[sp] = first;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            out.push(stack[sp]);
        }
        // Add oldcode + first as the next table entry.
        if (avail as usize) < 4096 {
            prefix[avail as usize] = oldcode as u16;
            suffix[avail as usize] = first;
            avail += 1;
            if avail == (1u16 << code_size) && code_size < 12 {
                code_size += 1;
            }
        }
        oldcode = code as i32;
        if out.len() >= out_len {
            break;
        }
    }
    out.resize(out_len, 0);
    Some(out)
}

/// GIF interlace pass row order: 0/8, 4/8, 2/4, 1/2.
fn deinterlace(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; w * h];
    let mut row = 0usize;
    for &(start, step) in &[(0usize, 8usize), (4, 8), (2, 4), (1, 2)] {
        let mut y = start;
        while y < h {
            dst[y * w..y * w + w].copy_from_slice(&src[row * w..row * w + w]);
            row += 1;
            y += step;
        }
    }
    dst
}

pub fn decode(data: &[u8]) -> Option<Gif> {
    let mut r = Reader { d: data, pos: 0 };
    let magic = r.take(6)?;
    if &magic[0..3] != b"GIF" || (magic != b"GIF89a" && magic != b"GIF87a") {
        return None;
    }
    let sw = r.u16()? as usize;
    let sh = r.u16()? as usize;
    if sw == 0 || sh == 0 || sw > MAX_DIM || sh > MAX_DIM {
        return None;
    }
    let packed = r.u8()?;
    let bg_index = r.u8()? as usize;
    let _aspect = r.u8()?;
    let gct = if packed & 0x80 != 0 {
        Some(read_color_table(&mut r, 2 << (packed & 7))?)
    } else {
        None
    };

    let bg = gct
        .as_ref()
        .and_then(|t| t.get(bg_index).copied())
        .unwrap_or(0xff00_0000);

    let mut canvas = vec![bg; sw * sh];
    let mut frames: Vec<GifFrame> = Vec::new();
    let mut pixmem = 0usize;

    // Pending Graphic Control Extension state for the next image.
    let mut delay_cs = 0u16;
    let mut transparent: Option<u8> = None;
    let mut disposal = 0u8;

    loop {
        match r.u8()? {
            0x3B => break, // trailer
            0x21 => {
                // Extension.
                match r.u8()? {
                    0xF9 => {
                        // Graphic Control Extension.
                        let n = r.u8()?; // block size, always 4
                        if n != 4 {
                            r.pos -= 1;
                            r.skip_sub_blocks()?;
                            continue;
                        }
                        let flags = r.u8()?;
                        delay_cs = r.u16()?;
                        let ti = r.u8()?;
                        let _term = r.u8()?;
                        disposal = (flags >> 2) & 7;
                        transparent = if flags & 1 != 0 { Some(ti) } else { None };
                    }
                    _ => {
                        // Application / comment / plain-text: skip its blocks.
                        r.skip_sub_blocks()?;
                    }
                }
            }
            0x2C => {
                // Image descriptor.
                let ix = r.u16()? as usize;
                let iy = r.u16()? as usize;
                let iw = r.u16()? as usize;
                let ih = r.u16()? as usize;
                let ipacked = r.u8()?;
                let lct = if ipacked & 0x80 != 0 {
                    Some(read_color_table(&mut r, 2 << (ipacked & 7))?)
                } else {
                    None
                };
                let table = lct.as_ref().or(gct.as_ref())?;
                let interlaced = ipacked & 0x40 != 0;
                let min_code_size = r.u8()?;
                let lzw = r.sub_blocks()?;
                if iw == 0 || ih == 0 || ix + iw > sw || iy + ih > sh {
                    // Out-of-bounds frame: ignore but keep parsing.
                    continue;
                }
                let mut idx = lzw_decode(&lzw, min_code_size, iw * ih)?;
                if interlaced {
                    idx = deinterlace(&idx, iw, ih);
                }
                // Composite onto the canvas.
                for y in 0..ih {
                    for x in 0..iw {
                        let pi = idx[y * iw + x];
                        if Some(pi) == transparent {
                            continue;
                        }
                        let color = table.get(pi as usize).copied().unwrap_or(0xff00_0000);
                        canvas[(iy + y) * sw + ix + x] = color;
                    }
                }
                pixmem += sw * sh;
                frames.push(GifFrame { delay_cs, pixels: canvas.clone() });
                // Apply disposal for the NEXT frame.
                match disposal {
                    2 => {
                        // Restore the image rect to background.
                        for y in 0..ih {
                            for x in 0..iw {
                                canvas[(iy + y) * sw + ix + x] = bg;
                            }
                        }
                    }
                    _ => {} // 0/1 leave; 3 (restore-previous) approximated as leave
                }
                // Reset per-image GCE state.
                delay_cs = 0;
                transparent = None;
                if frames.len() >= MAX_FRAMES || pixmem >= MAX_PIXMEM {
                    break;
                }
            }
            _ => return None, // unknown block
        }
    }

    if frames.is_empty() {
        return None;
    }
    kprintln!("GIF: {sw}x{sh}, {} frames", frames.len());
    Some(Gif { w: sw as u16, h: sh as u16, frames })
}
