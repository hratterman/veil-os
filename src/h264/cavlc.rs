//! CAVLC residual block decoding (ITU-T H.264 §9.2). Reads coeff_token, levels,
//! total_zeros and run_before using the generated VLC tables, returning the 4×4
//! coefficient levels in zig-zag scan order.

use super::bits::BitReader;
use super::cavlc_tables as ct;

/// Generic prefix-matched VLC read over a (len, bits) table slice of `n`
/// symbols. Returns the symbol index, or usize::MAX on failure.
fn read_vlc(b: &mut BitReader, lens: &[u8], codes: &[u16], n: usize) -> usize {
    let mut code = 0u32;
    let mut len = 0u8;
    loop {
        code = (code << 1) | b.bit();
        len += 1;
        for sym in 0..n {
            if lens[sym] == len && codes[sym] as u32 == code {
                return sym;
            }
        }
        if len >= 24 {
            return usize::MAX;
        }
    }
}

/// Decode coeff_token for a block with neighbour coeff count `nc` (or -1 for
/// chroma DC). Returns (total_coeff, trailing_ones).
fn coeff_token(b: &mut BitReader, nc: i32) -> (usize, usize) {
    let sym = if nc == -1 {
        read_vlc(b, &ct::CHROMA_DC_COEFF_TOKEN_LEN, &ct::CHROMA_DC_COEFF_TOKEN_BITS, 20)
    } else {
        let cat = if nc < 2 {
            0
        } else if nc < 4 {
            1
        } else if nc < 8 {
            2
        } else {
            3
        };
        let base = cat * 68;
        read_vlc(b, &ct::COEFF_TOKEN_LEN[base..base + 68], &ct::COEFF_TOKEN_BITS[base..base + 68], 68)
    };
    if sym == usize::MAX {
        return (0, 0);
    }
    (sym / 4, sym % 4)
}

fn read_total_zeros(b: &mut BitReader, total_coeff: usize, max_coeff: usize) -> usize {
    if max_coeff == 4 {
        // chroma DC (4:2:0)
        let row = (total_coeff - 1) * 4;
        let sym = read_vlc(b, &ct::CHROMA_DC_TOTAL_ZEROS_LEN[row..row + 4], &ct::CHROMA_DC_TOTAL_ZEROS_BITS[row..row + 4], 4);
        if sym == usize::MAX { 0 } else { sym }
    } else {
        let row = (total_coeff - 1) * 16;
        let sym = read_vlc(b, &ct::TOTAL_ZEROS_LEN[row..row + 16], &ct::TOTAL_ZEROS_BITS[row..row + 16], 16);
        if sym == usize::MAX { 0 } else { sym }
    }
}

fn read_run_before(b: &mut BitReader, zeros_left: usize) -> usize {
    let idx = zeros_left.min(7) - 1;
    let row = idx * 16;
    let sym = read_vlc(b, &ct::RUN_LEN[row..row + 16], &ct::RUN_BITS[row..row + 16], 16);
    if sym == usize::MAX { 0 } else { sym }
}

fn read_level_prefix(b: &mut BitReader) -> u32 {
    let mut n = 0u32;
    while b.bit() == 0 {
        n += 1;
        if n > 60 {
            break;
        }
    }
    n
}

/// Decode one residual block. `nc` is the predicted coefficient count (-1 for
/// chroma DC). `max_coeff` is 16, 15 (AC of I16x16), or 4 (chroma DC). Returns
/// (coeff_levels in scan order, total_coeff).
pub fn residual_block(b: &mut BitReader, nc: i32, max_coeff: usize) -> ([i32; 16], usize) {
    let mut coeff = [0i32; 16];
    let (total_coeff, trailing_ones) = coeff_token(b, nc);
    if total_coeff == 0 {
        return (coeff, 0);
    }
    let mut level = [0i32; 16];
    let mut suffix_length = if total_coeff > 10 && trailing_ones < 3 { 1u32 } else { 0 };
    for i in 0..total_coeff {
        if i < trailing_ones {
            level[i] = if b.bit() == 1 { -1 } else { 1 };
        } else {
            let level_prefix = read_level_prefix(b);
            let mut level_code = level_prefix.min(15) << suffix_length;
            let suffix_size = if level_prefix == 14 && suffix_length == 0 {
                4
            } else if level_prefix >= 15 {
                level_prefix - 3
            } else {
                suffix_length
            };
            if suffix_size > 0 {
                level_code += b.bits(suffix_size);
            }
            if level_prefix >= 15 && suffix_length == 0 {
                level_code += 15;
            }
            if level_prefix >= 16 {
                let sh = (level_prefix - 3).min(31);
                level_code += (1u32 << sh).wrapping_sub(4096);
            }
            if i == trailing_ones && trailing_ones < 3 {
                level_code += 2;
            }
            let lc = level_code as i32;
            level[i] = if lc % 2 == 0 { (lc + 2) >> 1 } else { (-lc - 1) >> 1 };
            if suffix_length == 0 {
                suffix_length = 1;
            }
            if level[i].unsigned_abs() > (3u32 << (suffix_length - 1)) && suffix_length < 6 {
                suffix_length += 1;
            }
        }
    }
    let total_zeros = if total_coeff < max_coeff {
        read_total_zeros(b, total_coeff, max_coeff)
    } else {
        0
    };
    // Distribute runs.
    let mut zeros_left = total_zeros;
    let mut runs = [0usize; 16];
    for i in 0..total_coeff.saturating_sub(1) {
        let run = if zeros_left > 0 {
            read_run_before(b, zeros_left)
        } else {
            0
        };
        runs[i] = run;
        zeros_left = zeros_left.saturating_sub(run);
    }
    runs[total_coeff - 1] = zeros_left;
    // Place levels into scan positions.
    let mut coeff_num = -1i32;
    for i in (0..total_coeff).rev() {
        coeff_num += runs[i] as i32 + 1;
        if (coeff_num as usize) < 16 {
            coeff[coeff_num as usize] = level[i];
        }
    }
    (coeff, total_coeff)
}
