//! H.264 inverse 4×4 integer transform, dequantization, and the Hadamard DC
//! transforms for Intra_16×16 luma DC and chroma DC. Follows ITU-T H.264
//! §8.5.10–8.5.12 (flat default scaling lists).

/// normAdjust4x4 V-table [qP%6][position-class].
const V: [[i32; 3]; 6] = [
    [10, 16, 13],
    [11, 18, 14],
    [13, 20, 16],
    [14, 23, 18],
    [16, 25, 20],
    [18, 29, 23],
];

/// Position class for (row, col) in a 4×4: 0 for even/even corners, 1 for the
/// odd/odd diagonal, 2 otherwise.
fn pos_class(i: usize, j: usize) -> usize {
    let ei = i % 2 == 0;
    let ej = j % 2 == 0;
    if ei && ej {
        0
    } else if !ei && !ej {
        1
    } else {
        2
    }
}

/// LevelScale4x4 = weightScale(16, flat) * normAdjust.
fn level_scale(m: usize, i: usize, j: usize) -> i32 {
    16 * V[m][pos_class(i, j)]
}

/// Dequantize a 4×4 residual block (raster order). If `dc` is Some, that value
/// is the already-scaled DC (from a Hadamard pass) and replaces position 0
/// without re-scaling.
pub fn dequant_4x4(c: &[i32; 16], qp: i32, dc: Option<i32>) -> [i32; 16] {
    let m = (qp % 6) as usize;
    let shift = qp / 6;
    let mut d = [0i32; 16];
    for i in 0..4 {
        for j in 0..4 {
            let k = i * 4 + j;
            if k == 0 && dc.is_some() {
                d[0] = dc.unwrap();
                continue;
            }
            let ls = level_scale(m, i, j) as i64;
            let ck = (c[k] as i64).clamp(-(1 << 20), 1 << 20);
            d[k] = (if shift >= 4 {
                (ck * ls) << (shift - 4)
            } else {
                (ck * ls + (1 << (3 - shift))) >> (4 - shift)
            }) as i32;
        }
    }
    d
}

/// Inverse 4×4 core transform of a (dequantized) block, raster order in/out.
/// Output residual = (e + 32) >> 6.
pub fn idct_4x4(d: &[i32; 16]) -> [i32; 16] {
    let mut e = [0i32; 16];
    // Horizontal (rows).
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let p0 = d[i * 4];
        let p1 = d[i * 4 + 1];
        let p2 = d[i * 4 + 2];
        let p3 = d[i * 4 + 3];
        let t0 = p0 + p2;
        let t1 = p0 - p2;
        let t2 = (p1 >> 1) - p3;
        let t3 = p1 + (p3 >> 1);
        tmp[i * 4] = t0 + t3;
        tmp[i * 4 + 1] = t1 + t2;
        tmp[i * 4 + 2] = t1 - t2;
        tmp[i * 4 + 3] = t0 - t3;
    }
    // Vertical (cols).
    for j in 0..4 {
        let p0 = tmp[j];
        let p1 = tmp[4 + j];
        let p2 = tmp[8 + j];
        let p3 = tmp[12 + j];
        let t0 = p0 + p2;
        let t1 = p0 - p2;
        let t2 = (p1 >> 1) - p3;
        let t3 = p1 + (p3 >> 1);
        e[j] = (t0 + t3 + 32) >> 6;
        e[4 + j] = (t1 + t2 + 32) >> 6;
        e[8 + j] = (t1 - t2 + 32) >> 6;
        e[12 + j] = (t0 - t3 + 32) >> 6;
    }
    e
}

/// Inverse Hadamard for the 16 Intra_16×16 luma DC coefficients (raster 4×4),
/// then scaling, returning the 16 scaled DC values (one per 4×4 block).
pub fn luma_dc_transform(c: &[i32; 16], qp: i32) -> [i32; 16] {
    let mut f = [0i32; 16];
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let p0 = c[i * 4];
        let p1 = c[i * 4 + 1];
        let p2 = c[i * 4 + 2];
        let p3 = c[i * 4 + 3];
        let t0 = p0 + p2;
        let t1 = p0 - p2;
        let t2 = p1 - p3;
        let t3 = p1 + p3;
        tmp[i * 4] = t0 + t3;
        tmp[i * 4 + 1] = t1 + t2;
        tmp[i * 4 + 2] = t1 - t2;
        tmp[i * 4 + 3] = t0 - t3;
    }
    for j in 0..4 {
        let p0 = tmp[j];
        let p1 = tmp[4 + j];
        let p2 = tmp[8 + j];
        let p3 = tmp[12 + j];
        let t0 = p0 + p2;
        let t1 = p0 - p2;
        let t2 = p1 - p3;
        let t3 = p1 + p3;
        f[j] = t0 + t3;
        f[4 + j] = t1 + t2;
        f[8 + j] = t1 - t2;
        f[12 + j] = t0 - t3;
    }
    // Scale.
    let m = (qp % 6) as usize;
    let shift = qp / 6;
    let ls = level_scale(m, 0, 0);
    let mut out = [0i32; 16];
    for k in 0..16 {
        out[k] = if shift >= 6 {
            (f[k] * ls) << (shift - 6)
        } else {
            (f[k] * ls + (1 << (5 - shift))) >> (6 - shift)
        };
    }
    out
}

/// Inverse Hadamard + scaling for the 4 chroma DC coefficients (2×2).
pub fn chroma_dc_transform(c: &[i32; 4], qp: i32) -> [i32; 4] {
    // 2×2 Hadamard.
    let f0 = c[0] + c[1] + c[2] + c[3];
    let f1 = c[0] - c[1] + c[2] - c[3];
    let f2 = c[0] + c[1] - c[2] - c[3];
    let f3 = c[0] - c[1] - c[2] + c[3];
    let f = [f0, f1, f2, f3];
    let m = (qp % 6) as usize;
    let shift = qp / 6;
    let ls = level_scale(m, 0, 0);
    let mut out = [0i32; 4];
    for k in 0..4 {
        out[k] = ((f[k] * ls) << shift) >> 5;
    }
    out
}

/// Map qP_luma to qP_chroma per Table 8-15.
pub fn chroma_qp(qp_luma: i32) -> i32 {
    const MAP: [i32; 22] = [
        29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39, 39, 39,
    ];
    let q = qp_luma.clamp(0, 51);
    if q < 30 {
        q
    } else {
        MAP[(q - 30) as usize]
    }
}

/// The inverse 4×4 zig-zag scan (frame/progressive). Maps scan position to the
/// raster index within the 4×4 block.
pub const ZIGZAG_4X4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
