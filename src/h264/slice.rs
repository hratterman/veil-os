//! H.264 slice decoding: slice header, the macroblock layer (CAVLC residual,
//! intra/inter prediction, inverse transform), the in-loop deblocking filter,
//! and YCbCr→RGB output. Constrained-baseline scope: I and P slices, CAVLC,
//! single reference frame, no B/field/weighted prediction.

use super::bits::BitReader;
use super::cavlc::residual_block;
use super::transform as tf;
use super::{Frame, Pps, Sps};
use alloc::vec;
use alloc::vec::Vec;

const DEBLOCK: bool = true;

/// coded_block_pattern me(v) maps (ITU Table 9-4) for 4:2:0.
const GOLOMB_TO_INTRA_CBP: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];
const GOLOMB_TO_INTER_CBP: [u8; 48] = [
    0, 16, 1, 2, 4, 8, 32, 3, 5, 10, 12, 15, 47, 7, 11, 13, 14, 6, 9, 31, 35, 37, 42, 44, 33, 34,
    36, 40, 39, 43, 45, 46, 17, 18, 20, 24, 19, 21, 26, 28, 23, 27, 29, 30, 22, 25, 38, 41,
];

/// 4×4 luma block scan → (bx, by) in 4×4 units within the macroblock.
const BLK_XY: [(usize, usize); 16] = [
    (0, 0), (1, 0), (0, 1), (1, 1), (2, 0), (3, 0), (2, 1), (3, 1),
    (0, 2), (1, 2), (0, 3), (1, 3), (2, 2), (3, 2), (2, 3), (3, 3),
];

fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

pub struct Decoder {
    sps: Option<Sps>,
    pps: Option<Pps>,
    w_mbs: usize,
    h_mbs: usize,
    width: usize,
    height: usize,
    // Current picture planes.
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    // Reference picture (previous decoded frame).
    ref_y: Vec<u8>,
    ref_cb: Vec<u8>,
    ref_cr: Vec<u8>,
    has_ref: bool,
    // Per 4×4-luma-block frame grids.
    nnz_y: Vec<u8>,   // (w_mbs*4) × (h_mbs*4)
    mvx: Vec<i16>,
    mvy: Vec<i16>,
    refidx: Vec<i8>,  // -1 intra
    i4mode: Vec<i8>,  // intra4x4 pred mode contribution (or 2)
    i4set: Vec<bool>, // per 4×4 block: is its intra4x4 pred mode determined yet?
    // Per 4×4-chroma-block grids.
    nnz_c: [Vec<u8>; 2], // (w_mbs*2) × (h_mbs*2)
    // Per-macroblock.
    mb_intra: Vec<bool>,
    mb_qp: Vec<i8>,
    mb_type16: Vec<i8>, // -1 not I16x16; else pred mode 0..3 (for deblock bS)
    decoded4: Vec<bool>, // per 4×4 luma block: reconstructed yet? (neighbor avail)
    g4w: usize,          // width in 4×4 luma blocks  (= w_mbs*4)
    g2w: usize,          // width in 4×4 chroma blocks (= w_mbs*2)
    qp_prev: i32,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            sps: None,
            pps: None,
            w_mbs: 0,
            h_mbs: 0,
            width: 0,
            height: 0,
            y: Vec::new(),
            cb: Vec::new(),
            cr: Vec::new(),
            ref_y: Vec::new(),
            ref_cb: Vec::new(),
            ref_cr: Vec::new(),
            has_ref: false,
            nnz_y: Vec::new(),
            mvx: Vec::new(),
            mvy: Vec::new(),
            refidx: Vec::new(),
            i4mode: Vec::new(),
            i4set: Vec::new(),
            nnz_c: [Vec::new(), Vec::new()],
            mb_intra: Vec::new(),
            mb_qp: Vec::new(),
            mb_type16: Vec::new(),
            decoded4: Vec::new(),
            g4w: 0,
            g2w: 0,
            qp_prev: 26,
        }
    }

    pub fn set_sps(&mut self, sps: Sps) {
        self.w_mbs = sps.width_mbs();
        self.h_mbs = sps.height_mbs();
        self.width = sps.width();
        self.height = sps.height();
        self.sps = Some(sps);
        self.alloc();
    }

    pub fn set_pps(&mut self, pps: Pps) {
        self.pps = Some(pps);
    }

    fn alloc(&mut self) {
        let (yw, yh) = (self.w_mbs * 16, self.h_mbs * 16);
        let (cw, ch) = (self.w_mbs * 8, self.h_mbs * 8);
        self.y = vec![0; yw * yh];
        self.cb = vec![128; cw * ch];
        self.cr = vec![128; cw * ch];
        self.ref_y = vec![0; yw * yh];
        self.ref_cb = vec![128; cw * ch];
        self.ref_cr = vec![128; cw * ch];
        let n4 = (self.w_mbs * 4) * (self.h_mbs * 4);
        self.nnz_y = vec![0; n4];
        self.mvx = vec![0; n4];
        self.mvy = vec![0; n4];
        self.refidx = vec![-1; n4];
        self.i4mode = vec![2; n4];
        self.i4set = vec![false; n4];
        let n2 = (self.w_mbs * 2) * (self.h_mbs * 2);
        self.nnz_c = [vec![0; n2], vec![0; n2]];
        let nmb = self.w_mbs * self.h_mbs;
        self.mb_intra = vec![true; nmb];
        self.mb_qp = vec![26; nmb];
        self.mb_type16 = vec![-1; nmb];
        self.decoded4 = vec![false; n4];
        self.g4w = self.w_mbs * 4;
        self.g2w = self.w_mbs * 2;
    }

    /// Handle a NAL unit. Returns a decoded Frame for slice NALs (one slice =
    /// one frame in our content), else None.
    pub fn handle_nal(&mut self, _ref_idc: u8, ty: u8, rbsp: &[u8]) -> Option<Frame> {
        match ty {
            7 => {
                self.set_sps(super::parse_sps(rbsp));
                None
            }
            8 => {
                self.set_pps(super::parse_pps(rbsp));
                None
            }
            1 | 5 => self.decode_slice(rbsp, ty == 5),
            _ => None,
        }
    }

    fn decode_slice(&mut self, rbsp: &[u8], idr: bool) -> Option<Frame> {
        let sps = self.sps.clone()?;
        let pps = self.pps.clone()?;
        let mut b = BitReader::new(rbsp);
        let _first_mb = b.ue();
        let slice_type = b.ue() % 5; // 0=P,1=B,2=I,3=SP,4=SI
        let _pps_id = b.ue();
        b.bits(sps.log2_max_frame_num); // frame_num
        if idr {
            b.ue(); // idr_pic_id
        }
        if sps.pic_order_cnt_type == 0 {
            b.bits(sps.log2_max_poc_lsb); // pic_order_cnt_lsb
            if pps.pic_order_present {
                b.se();
            }
        }
        let is_p = slice_type == 0;
        if is_p {
            // num_ref_idx_active_override_flag
            if b.bit() == 1 {
                b.ue(); // num_ref_idx_l0_active_minus1
            }
            // ref_pic_list_modification_flag_l0
            if b.bit() == 1 {
                loop {
                    let idc = b.ue();
                    if idc == 3 {
                        break;
                    }
                    b.ue(); // abs_diff_pic_num / long_term_pic_num
                }
            }
        }
        // dec_ref_pic_marking (nal_ref_idc != 0 for slices we care about).
        if idr {
            b.bit(); // no_output_of_prior_pics
            b.bit(); // long_term_reference_flag
        } else {
            // adaptive_ref_pic_marking_mode_flag
            if b.bit() == 1 {
                loop {
                    let op = b.ue();
                    if op == 0 {
                        break;
                    }
                    if op == 1 || op == 3 {
                        b.ue();
                    }
                    if op == 2 {
                        b.ue();
                    }
                    if op == 3 || op == 6 {
                        b.ue();
                    }
                    if op == 4 {
                        b.ue();
                    }
                }
            }
        }
        let slice_qp = pps.pic_init_qp + b.se();
        self.qp_prev = slice_qp;
        // deblocking filter control
        let mut disable_deblock = 0u32;
        let mut alpha_off = 0i32;
        let mut beta_off = 0i32;
        if pps.deblocking_filter_control_present {
            disable_deblock = b.ue();
            if disable_deblock != 1 {
                alpha_off = b.se() * 2;
                beta_off = b.se() * 2;
            }
        }

        // Reset per-frame grids.
        for v in self.nnz_y.iter_mut() { *v = 0; }
        for v in self.nnz_c[0].iter_mut() { *v = 0; }
        for v in self.nnz_c[1].iter_mut() { *v = 0; }
        for v in self.mvx.iter_mut() { *v = 0; }
        for v in self.mvy.iter_mut() { *v = 0; }
        for v in self.refidx.iter_mut() { *v = -1; }
        for v in self.i4mode.iter_mut() { *v = 2; }
        for v in self.i4set.iter_mut() { *v = false; }
        for v in self.mb_intra.iter_mut() { *v = true; }
        for v in self.mb_type16.iter_mut() { *v = -1; }
        for v in self.decoded4.iter_mut() { *v = false; }

        // Macroblock loop. In a P slice each coded MB is preceded by an
        // mb_skip_run giving the count of P_Skip MBs before it.
        let nmb = self.w_mbs * self.h_mbs;
        let mut mb_addr = 0usize;
        while mb_addr < nmb {
            if is_p {
                let skip_run = b.ue();
                for _ in 0..skip_run {
                    if mb_addr >= nmb {
                        break;
                    }
                    self.decode_pskip(mb_addr);
                    mb_addr += 1;
                }
                if mb_addr >= nmb || !b.more_rbsp_data() {
                    break;
                }
            }
            self.decode_mb(&mut b, mb_addr, is_p);
            mb_addr += 1;
        }

        if DEBLOCK && disable_deblock != 1 {
            self.deblock_frame(alpha_off, beta_off);
        }

        let frame = self.to_rgb();
        // The decoded picture becomes the reference for subsequent P slices.
        core::mem::swap(&mut self.y, &mut self.ref_y);
        core::mem::swap(&mut self.cb, &mut self.ref_cb);
        core::mem::swap(&mut self.cr, &mut self.ref_cr);
        self.has_ref = true;
        Some(frame)
    }

    // ---- grid coordinate helpers --------------------------------------------

    fn mb_xy(&self, addr: usize) -> (usize, usize) {
        (addr % self.w_mbs, addr / self.w_mbs)
    }

    // ---- P_Skip --------------------------------------------------------------

    fn decode_pskip(&mut self, addr: usize) {
        let (mbx, mby) = self.mb_xy(addr);
        self.mb_intra[addr] = false;
        self.mb_type16[addr] = -1;
        self.mb_qp[addr] = self.qp_prev as i8;
        // Predicted mv for P_Skip.
        let (mvx, mvy) = self.pskip_mv(mbx, mby);
        self.store_mb_inter(mbx, mby, 0, 0, 4, 4, mvx, mvy, 0);
        self.mc_luma(mbx, mby, 0, 0, 16, 16, mvx, mvy);
        self.mc_chroma(mbx, mby, 0, 0, 8, 8, mvx, mvy);
        self.mark_modes_known(mbx, mby, false);
    }

    fn pskip_mv(&self, mbx: usize, mby: usize) -> (i16, i16) {
        // If A or B unavailable, or either is zero-mv ref0, predict (0,0).
        let a = self.neighbor_mv(mbx, mby, -1, 0);
        let bb = self.neighbor_mv(mbx, mby, 0, -1);
        let a_avail = mbx > 0;
        let b_avail = mby > 0;
        if !a_avail || !b_avail {
            return (0, 0);
        }
        if (a.2 == 0 && a.0 == 0 && a.1 == 0) || (bb.2 == 0 && bb.0 == 0 && bb.1 == 0) {
            return (0, 0);
        }
        self.predict_mv(mbx, mby, 0, 0, 4, 4, 0)
    }

    // ---- macroblock decode ---------------------------------------------------

    fn decode_mb(&mut self, b: &mut BitReader, addr: usize, is_p: bool) {
        let (mbx, mby) = self.mb_xy(addr);
        let mut mb_type = b.ue();
        // In P slices, mb_type >= 5 means an intra MB (subtract 5).
        let mut intra = true;
        if is_p {
            if mb_type < 5 {
                intra = false;
            } else {
                mb_type -= 5;
            }
        }
        self.mb_intra[addr] = intra;

        if intra {
            self.decode_mb_intra(b, addr, mbx, mby, mb_type);
        } else {
            self.decode_mb_inter(b, addr, mbx, mby, mb_type);
        }
        self.mark_modes_known(mbx, mby, intra);
    }

    /// Mark this MB's 4×4 blocks as having a known intra4×4 mode for neighbour
    /// mode prediction. I4×4 blocks set their real modes during reading; others
    /// contribute DC (2) when not constrained-intra.
    fn mark_modes_known(&mut self, mbx: usize, mby: usize, intra: bool) {
        let constrained = self.pps.as_ref().map(|p| p.constrained_intra_pred).unwrap_or(false);
        if !intra && constrained {
            return; // inter neighbour unavailable for constrained intra prediction
        }
        let g4w = self.g4w;
        for j in 0..4 {
            for i in 0..4 {
                let idx = (mby * 4 + j) * g4w + (mbx * 4 + i);
                if !self.i4set[idx] {
                    self.i4set[idx] = true;
                    self.i4mode[idx] = 2;
                }
            }
        }
    }

    // ---- intra ---------------------------------------------------------------

    fn decode_mb_intra(&mut self, b: &mut BitReader, addr: usize, mbx: usize, mby: usize, mb_type: u32) {
        // Mark inter grids as intra.
        self.set_mb_refidx(mbx, mby, -1);
        if mb_type == 0 {
            // I_NxN (I_4x4).
            self.mb_type16[addr] = -1;
            // transform_size_8x8 not in baseline.
            let mut modes = [0i8; 16];
            for blk in 0..16 {
                let (bx, by) = BLK_XY[blk];
                let pred = self.pred_i4_mode(mbx, mby, bx, by);
                let prev = b.bit();
                let mode = if prev == 1 {
                    pred
                } else {
                    let rem = b.bits(3) as i8;
                    if rem < pred { rem } else { rem + 1 }
                };
                modes[blk] = mode;
                // store immediately so later blocks predict from it
                let (ax, ay) = (mbx * 4 + bx, mby * 4 + by);
                self.i4mode[ay * self.g4w + ax] = mode;
                self.i4set[ay * self.g4w + ax] = true;
            }
            let chroma_mode = b.ue();
            let cbp_code = b.ue() as usize;
            let cbp = GOLOMB_TO_INTRA_CBP[cbp_code.min(47)] as u32;
            let cbp_luma = cbp & 15;
            let cbp_chroma = cbp >> 4;
            self.update_qp(b, addr, cbp);
            let qp = self.mb_qp[addr] as i32;
            // Reconstruct each 4×4 with intra prediction then residual.
            for blk in 0..16 {
                let (bx, by) = BLK_XY[blk];
                self.intra4x4_predict(mbx, mby, bx, by, modes[blk] as u32);
                let nnz = if cbp_luma & (1 << (blk / 4)) != 0 {
                    let nc = self.luma_nc(mbx, mby, bx, by);
                    let (coeffs, tc) = residual_block(b, nc, 16);
                    self.add_residual_4x4(mbx, mby, bx, by, &coeffs, qp, None);
                    tc as u8
                } else {
                    0
                };
                let (ax, ay) = (mbx * 4 + bx, mby * 4 + by);
                self.nnz_y[ay * self.g4w + ax] = nnz;
                self.decoded4[ay * self.g4w + ax] = true; // available for next block's pred
            }
            self.chroma_decode(b, addr, mbx, mby, chroma_mode, cbp_chroma, qp, true);
            let _ = modes;
        } else if mb_type == 25 {
            // I_PCM: byte-aligned raw samples.
            self.decode_ipcm(b, addr, mbx, mby);
        } else {
            // I_16x16.
            let t = mb_type - 1;
            let pred_mode = (t % 4) as u32;
            let cbp_chroma = (t / 4) % 3;
            let cbp_luma = if t / 12 != 0 { 15u32 } else { 0 };
            self.mb_type16[addr] = pred_mode as i8;
            let chroma_mode = b.ue();
            let cbp = (cbp_chroma << 4) | cbp_luma;
            self.update_qp(b, addr, cbp | if true { 1 << 30 } else { 0 }); // I16x16 always reads mb_qp_delta
            let qp = self.mb_qp[addr] as i32;
            // Luma 16×16 prediction.
            self.intra16x16_predict(mbx, mby, pred_mode);
            // Luma DC.
            let nc = self.luma_nc(mbx, mby, 0, 0);
            let (dc_coeffs, _) = residual_block(b, nc, 16);
            // inverse scan DC to raster 4×4
            let mut dc_raster = [0i32; 16];
            for k in 0..16 {
                dc_raster[tf::ZIGZAG_4X4[k]] = dc_coeffs[k];
            }
            let dc = tf::luma_dc_transform(&dc_raster, qp);
            // Luma AC blocks.
            for blk in 0..16 {
                let (bx, by) = BLK_XY[blk];
                let block_dc = dc[by * 4 + bx];
                let nnz = if cbp_luma != 0 {
                    let nc = self.luma_nc(mbx, mby, bx, by);
                    let (mut coeffs, tc) = residual_block(b, nc, 15);
                    // residual_block for AC returns 16 entries with [0]=first AC;
                    // shift so scan index 0 stays DC(0) and AC fill 1..16.
                    let mut ac = [0i32; 16];
                    for k in 0..15 {
                        ac[k + 1] = coeffs[k];
                    }
                    coeffs = ac;
                    self.add_residual_4x4(mbx, mby, bx, by, &coeffs, qp, Some(block_dc));
                    tc as u8
                } else {
                    // DC-only block.
                    let coeffs = [0i32; 16];
                    self.add_residual_4x4(mbx, mby, bx, by, &coeffs, qp, Some(block_dc));
                    0
                };
                let (ax, ay) = (mbx * 4 + bx, mby * 4 + by);
                self.nnz_y[ay * self.g4w + ax] = nnz;
                self.decoded4[ay * self.g4w + ax] = true;
            }
            self.chroma_decode(b, addr, mbx, mby, chroma_mode, cbp_chroma, qp, true);
        }
    }

    fn decode_ipcm(&mut self, b: &mut BitReader, addr: usize, mbx: usize, mby: usize) {
        // align to byte
        while !b.byte_aligned() {
            b.bit();
        }
        let ys = self.w_mbs * 16;
        for j in 0..16 {
            for i in 0..16 {
                let v = b.bits(8) as u8;
                let (px, py) = (mbx * 16 + i, mby * 16 + j);
                self.y[py * ys + px] = v;
            }
        }
        let cs = self.w_mbs * 8;
        for comp in 0..2 {
            for j in 0..8 {
                for i in 0..8 {
                    let v = b.bits(8) as u8;
                    let (px, py) = (mbx * 8 + i, mby * 8 + j);
                    if comp == 0 {
                        self.cb[py * cs + px] = v;
                    } else {
                        self.cr[py * cs + px] = v;
                    }
                }
            }
        }
        self.mb_qp[addr] = 0;
        // nnz treated as max (16) for deblocking strength.
        for blk in 0..16 {
            let (bx, by) = BLK_XY[blk];
            let (ax, ay) = (mbx * 4 + bx, mby * 4 + by);
            self.nnz_y[ay * self.g4w + ax] = 16;
            self.decoded4[ay * self.g4w + ax] = true;
        }
    }

    /// In-loop deblocking filter (ITU §8.7). Filters vertical then horizontal
    /// edges of every macroblock (luma + chroma), with boundary strengths from
    /// intra/coeff/motion differences.
    fn deblock_frame(&mut self, alpha_off: i32, beta_off: i32) {
        let off = self.pps.as_ref().map(|p| p.chroma_qp_index_offset).unwrap_or(0);
        for mby in 0..self.h_mbs {
            for mbx in 0..self.w_mbs {
                // Vertical edges (left→right): e=0 is the left MB boundary.
                for e in 0..4 {
                    if e == 0 && mbx == 0 {
                        continue;
                    }
                    let bs = self.edge_bs(mbx, mby, e, true);
                    if bs.iter().all(|&b| b == 0) {
                        continue;
                    }
                    let qpp = self.mb_qp[mby * self.w_mbs + (if e == 0 { mbx - 1 } else { mbx })] as i32;
                    let qpq = self.mb_qp[mby * self.w_mbs + mbx] as i32;
                    let qpav = (qpp + qpq + 1) >> 1;
                    // Luma.
                    let stride = self.w_mbs * 16;
                    let x = mbx * 16 + e * 4;
                    for k in 0..4 {
                        if bs[k] == 0 {
                            continue;
                        }
                        let (alpha, beta, tc0) = thresholds(qpav, bs[k], alpha_off, beta_off);
                        if alpha == 0 {
                            continue;
                        }
                        for r in 0..4 {
                            let y = mby * 16 + k * 4 + r;
                            filter_line(&mut self.y, y * stride + x, 1, bs[k], alpha, beta, tc0, false);
                        }
                    }
                    // Chroma at luma edges 0 and 2.
                    if e == 0 || e == 2 {
                        let cstride = self.w_mbs * 8;
                        let cx = mbx * 8 + if e == 0 { 0 } else { 4 };
                        let qpcav = (tf::chroma_qp(qpp + off) + tf::chroma_qp(qpq + off) + 1) >> 1;
                        for cr in 0..8 {
                            let b = bs[cr / 2];
                            if b == 0 {
                                continue;
                            }
                            let (alpha, beta, tc0) = thresholds(qpcav, b, alpha_off, beta_off);
                            if alpha == 0 {
                                continue;
                            }
                            let y = mby * 8 + cr;
                            filter_line(&mut self.cb, y * cstride + cx, 1, b, alpha, beta, tc0, true);
                            filter_line(&mut self.cr, y * cstride + cx, 1, b, alpha, beta, tc0, true);
                        }
                    }
                }
                // Horizontal edges (top→bottom): e=0 is the top MB boundary.
                for e in 0..4 {
                    if e == 0 && mby == 0 {
                        continue;
                    }
                    let bs = self.edge_bs(mbx, mby, e, false);
                    if bs.iter().all(|&b| b == 0) {
                        continue;
                    }
                    let qpp = self.mb_qp[(if e == 0 { mby - 1 } else { mby }) * self.w_mbs + mbx] as i32;
                    let qpq = self.mb_qp[mby * self.w_mbs + mbx] as i32;
                    let qpav = (qpp + qpq + 1) >> 1;
                    let stride = self.w_mbs * 16;
                    let y = mby * 16 + e * 4;
                    for k in 0..4 {
                        if bs[k] == 0 {
                            continue;
                        }
                        let (alpha, beta, tc0) = thresholds(qpav, bs[k], alpha_off, beta_off);
                        if alpha == 0 {
                            continue;
                        }
                        for r in 0..4 {
                            let x = mbx * 16 + k * 4 + r;
                            filter_line(&mut self.y, y * stride + x, stride, bs[k], alpha, beta, tc0, false);
                        }
                    }
                    if e == 0 || e == 2 {
                        let cstride = self.w_mbs * 8;
                        let cy = mby * 8 + if e == 0 { 0 } else { 4 };
                        let qpcav = (tf::chroma_qp(qpp + off) + tf::chroma_qp(qpq + off) + 1) >> 1;
                        for cc in 0..8 {
                            let b = bs[cc / 2];
                            if b == 0 {
                                continue;
                            }
                            let (alpha, beta, tc0) = thresholds(qpcav, b, alpha_off, beta_off);
                            if alpha == 0 {
                                continue;
                            }
                            let x = mbx * 8 + cc;
                            filter_line(&mut self.cb, cy * cstride + x, cstride, b, alpha, beta, tc0, true);
                            filter_line(&mut self.cr, cy * cstride + x, cstride, b, alpha, beta, tc0, true);
                        }
                    }
                }
            }
        }
    }

    /// Boundary strengths (one per 4-sample segment) for the edge `e` of MB
    /// (mbx,mby); `vertical` selects a vertical (column) edge vs horizontal.
    fn edge_bs(&self, mbx: usize, mby: usize, e: usize, vertical: bool) -> [u8; 4] {
        let mut bs = [0u8; 4];
        let mb_bound = e == 0;
        for k in 0..4 {
            // q block (this side) and p block (other side) in 4×4 grid coords.
            let (qx, qy, px, py) = if vertical {
                (mbx * 4 + e, mby * 4 + k, mbx * 4 + e - 1, mby * 4 + k)
            } else {
                (mbx * 4 + k, mby * 4 + e, mbx * 4 + k, mby * 4 + e - 1)
            };
            bs[k] = self.bs_at(px, py, qx, qy, mb_bound);
        }
        bs
    }

    fn bs_at(&self, px: usize, py: usize, qx: usize, qy: usize, mb_bound: bool) -> u8 {
        let g4w = self.g4w;
        let pmb = (py / 4) * self.w_mbs + (px / 4);
        let qmb = (qy / 4) * self.w_mbs + (qx / 4);
        let p_intra = self.mb_intra[pmb];
        let q_intra = self.mb_intra[qmb];
        if p_intra || q_intra {
            return if mb_bound { 4 } else { 3 };
        }
        let pi = py * g4w + px;
        let qi = qy * g4w + qx;
        if self.nnz_y[pi] > 0 || self.nnz_y[qi] > 0 {
            return 2;
        }
        if self.refidx[pi] != self.refidx[qi]
            || (self.mvx[pi] - self.mvx[qi]).abs() >= 4
            || (self.mvy[pi] - self.mvy[qi]).abs() >= 4
        {
            return 1;
        }
        0
    }

    // ---- inter ---------------------------------------------------------------

    fn decode_mb_inter(&mut self, b: &mut BitReader, addr: usize, mbx: usize, mby: usize, mb_type: u32) {
        self.mb_type16[addr] = -1;
        // Partitions: (w,h) in 4×4 units, and how many.
        // mb_type: 0=16x16, 1=16x8, 2=8x16, 3=8x8, 4=8x8ref0
        let mut parts: Vec<(usize, usize, usize, usize)> = Vec::new(); // (bx,by,bw,bh) in 4x4 units
        let mut sub_types = [0u32; 4];
        match mb_type {
            0 => parts.push((0, 0, 4, 4)),
            1 => {
                parts.push((0, 0, 4, 2));
                parts.push((0, 2, 4, 2));
            }
            2 => {
                parts.push((0, 0, 2, 4));
                parts.push((2, 0, 2, 4));
            }
            _ => {
                // 8x8: read 4 sub_mb_types first.
                for st in sub_types.iter_mut() {
                    *st = b.ue();
                }
            }
        }

        // ref_idx: with a single reference, always 0. Baseline P has
        // num_ref_idx<=1 typically; read te(v) only if >1 (we assume 1).
        // mvd per partition.
        if mb_type <= 2 {
            for &(bx, by, bw, bh) in &parts.clone() {
                let (pmvx, pmvy) = self.predict_mv(mbx, mby, bx, by, bw, bh, 0);
                let mvdx = b.se();
                let mvdy = b.se();
                let mvx = (pmvx as i32 + mvdx) as i16;
                let mvy = (pmvy as i32 + mvdy) as i16;
                self.store_mb_inter(mbx, mby, bx, by, bw, bh, mvx, mvy, 0);
                self.mc_luma(mbx, mby, bx * 4, by * 4, bw * 4, bh * 4, mvx, mvy);
                self.mc_chroma(mbx, mby, bx * 2, by * 2, bw * 2, bh * 2, mvx, mvy);
            }
        } else {
            // 8x8: each 8×8 has sub-partitions.
            let sub_layout = |st: u32| -> (usize, usize, usize) {
                // returns (subW, subH, count) in 4×4 units
                match st {
                    0 => (2, 2, 1),
                    1 => (2, 1, 2),
                    2 => (1, 2, 2),
                    _ => (1, 1, 4),
                }
            };
            for i8 in 0..4 {
                let (ox, oy) = ((i8 % 2) * 2, (i8 / 2) * 2);
                let (sw, sh, cnt) = sub_layout(sub_types[i8]);
                for s in 0..cnt {
                    let (sx, sy) = match (sw, sh) {
                        (2, 2) => (0, 0),
                        (2, 1) => (0, s),
                        (1, 2) => (s, 0),
                        _ => (s % 2, s / 2),
                    };
                    let bx = ox + sx;
                    let by = oy + sy;
                    let (pmvx, pmvy) = self.predict_mv(mbx, mby, bx, by, sw, sh, 0);
                    let mvdx = b.se();
                    let mvdy = b.se();
                    let mvx = (pmvx as i32 + mvdx) as i16;
                    let mvy = (pmvy as i32 + mvdy) as i16;
                    self.store_mb_inter(mbx, mby, bx, by, sw, sh, mvx, mvy, 0);
                    self.mc_luma(mbx, mby, bx * 4, by * 4, sw * 4, sh * 4, mvx, mvy);
                    self.mc_chroma(mbx, mby, bx * 2, by * 2, sw * 2, sh * 2, mvx, mvy);
                }
            }
        }

        // Residual.
        let cbp_code = b.ue() as usize;
        let cbp = GOLOMB_TO_INTER_CBP[cbp_code.min(47)] as u32;
        let cbp_luma = cbp & 15;
        let cbp_chroma = cbp >> 4;
        self.update_qp(b, addr, cbp);
        let qp = self.mb_qp[addr] as i32;
        for blk in 0..16 {
            let (bx, by) = BLK_XY[blk];
            let nnz = if cbp_luma & (1 << (blk / 4)) != 0 {
                let nc = self.luma_nc(mbx, mby, bx, by);
                let (coeffs, tc) = residual_block(b, nc, 16);
                self.add_residual_4x4(mbx, mby, bx, by, &coeffs, qp, None);
                tc as u8
            } else {
                0
            };
            let (ax, ay) = (mbx * 4 + bx, mby * 4 + by);
            self.nnz_y[ay * self.g4w + ax] = nnz;
        }
        self.chroma_decode(b, addr, mbx, mby, 0, cbp_chroma, qp, false);
    }

    // ---- QP ------------------------------------------------------------------

    fn update_qp(&mut self, b: &mut BitReader, addr: usize, cbp_with_flag: u32) {
        let force = cbp_with_flag & (1 << 30) != 0;
        let cbp = cbp_with_flag & !(1 << 30);
        if cbp != 0 || force {
            let delta = b.se();
            self.qp_prev = (self.qp_prev + delta + 52).rem_euclid(52);
        }
        self.mb_qp[addr] = self.qp_prev as i8;
    }

    // ---- residual application ------------------------------------------------

    fn add_residual_4x4(&mut self, mbx: usize, mby: usize, bx: usize, by: usize, coeffs_scan: &[i32; 16], qp: i32, dc: Option<i32>) {
        // Inverse zig-zag to raster.
        let mut raster = [0i32; 16];
        for k in 0..16 {
            raster[tf::ZIGZAG_4X4[k]] = coeffs_scan[k];
        }
        let d = tf::dequant_4x4(&raster, qp, dc);
        let res = tf::idct_4x4(&d);
        let ys = self.w_mbs * 16;
        let (ox, oy) = (mbx * 16 + bx * 4, mby * 16 + by * 4);
        for j in 0..4 {
            for i in 0..4 {
                let idx = (oy + j) * ys + (ox + i);
                let v = self.y[idx] as i32 + res[j * 4 + i];
                self.y[idx] = clip(v);
            }
        }
    }

    fn add_chroma_residual_4x4(&mut self, comp: usize, mbx: usize, mby: usize, bx: usize, by: usize, coeffs_scan: &[i32; 16], qp: i32, dc: i32) {
        let mut raster = [0i32; 16];
        for k in 0..16 {
            raster[tf::ZIGZAG_4X4[k]] = coeffs_scan[k];
        }
        let d = tf::dequant_4x4(&raster, qp, Some(dc));
        let res = tf::idct_4x4(&d);
        let cs = self.w_mbs * 8;
        let (ox, oy) = (mbx * 8 + bx * 4, mby * 8 + by * 4);
        let plane = if comp == 0 { &mut self.cb } else { &mut self.cr };
        for j in 0..4 {
            for i in 0..4 {
                let idx = (oy + j) * cs + (ox + i);
                let v = plane[idx] as i32 + res[j * 4 + i];
                plane[idx] = clip(v);
            }
        }
    }

    // ---- chroma decode (pred + residual) -------------------------------------

    fn chroma_decode(&mut self, b: &mut BitReader, _addr: usize, mbx: usize, mby: usize, chroma_mode: u32, cbp_chroma: u32, qp_luma: i32, intra: bool) {
        if intra {
            self.intra_chroma_predict(mbx, mby, chroma_mode);
        }
        let qpc = tf::chroma_qp(qp_luma + self.pps.as_ref().map(|p| p.chroma_qp_index_offset).unwrap_or(0));
        // Chroma DC (2×2 per component).
        let mut dc = [[0i32; 4]; 2];
        if cbp_chroma & 3 != 0 {
            for c in 0..2 {
                let (coeffs, _tc) = residual_block(b, -1, 4);
                // first 4 scan coeffs are the 2×2 DC in raster order.
                let raw = [coeffs[0], coeffs[1], coeffs[2], coeffs[3]];
                dc[c] = tf::chroma_dc_transform(&raw, qpc);
            }
        }
        // Chroma AC.
        for c in 0..2 {
            for blk in 0..4 {
                let bx = blk % 2;
                let by = blk / 2;
                let block_dc = dc[c][by * 2 + bx];
                if cbp_chroma & 2 != 0 {
                    let nc = self.chroma_nc(c, mbx, mby, bx, by);
                    let (coeffs, tc) = residual_block(b, nc, 15);
                    let mut ac = [0i32; 16];
                    for k in 0..15 {
                        ac[k + 1] = coeffs[k];
                    }
                    self.add_chroma_residual_4x4(c, mbx, mby, bx, by, &ac, qpc, block_dc);
                    let (ax, ay) = (mbx * 2 + bx, mby * 2 + by);
                    self.nnz_c[c][ay * self.g2w + ax] = tc as u8;
                } else {
                    let coeffs = [0i32; 16];
                    self.add_chroma_residual_4x4(c, mbx, mby, bx, by, &coeffs, qpc, block_dc);
                    let (ax, ay) = (mbx * 2 + bx, mby * 2 + by);
                    self.nnz_c[c][ay * self.g2w + ax] = 0;
                }
            }
        }
    }

    // ---- nC computation ------------------------------------------------------

    fn luma_nc(&self, mbx: usize, mby: usize, bx: usize, by: usize) -> i32 {
        let ax = mbx * 4 + bx;
        let ay = mby * 4 + by;
        let a_avail = ax > 0;
        let b_avail = ay > 0;
        let na = if a_avail { self.nnz_y[ay * self.g4w + ax - 1] as i32 } else { -1 };
        let nb = if b_avail { self.nnz_y[(ay - 1) * self.g4w + ax] as i32 } else { -1 };
        combine_nc(na, nb)
    }

    fn chroma_nc(&self, comp: usize, mbx: usize, mby: usize, bx: usize, by: usize) -> i32 {
        let ax = mbx * 2 + bx;
        let ay = mby * 2 + by;
        let a_avail = ax > 0;
        let b_avail = ay > 0;
        let na = if a_avail { self.nnz_c[comp][ay * self.g2w + ax - 1] as i32 } else { -1 };
        let nb = if b_avail { self.nnz_c[comp][(ay - 1) * self.g2w + ax] as i32 } else { -1 };
        combine_nc(na, nb)
    }
}

fn combine_nc(na: i32, nb: i32) -> i32 {
    match (na >= 0, nb >= 0) {
        (true, true) => (na + nb + 1) >> 1,
        (true, false) => na,
        (false, true) => nb,
        (false, false) => 0,
    }
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    a + b + c - a.min(b).min(c) - a.max(b).max(c)
}

/// (alpha, beta, tc0) for a given averaged QP, boundary strength and offsets.
fn thresholds(qp_av: i32, bs: u8, alpha_off: i32, beta_off: i32) -> (i32, i32, i32) {
    use super::deblock_tables as dt;
    let ia = (qp_av + alpha_off).clamp(0, 51) as usize;
    let ib = (qp_av + beta_off).clamp(0, 51) as usize;
    let alpha = dt::ALPHA[ia] as i32;
    let beta = dt::BETA[ib] as i32;
    let tc0 = if bs >= 1 && bs <= 3 { dt::TC0[ia][(bs - 1) as usize] as i32 } else { 0 };
    (alpha, beta, tc0)
}

/// Filter one 8-sample line straddling an edge. `base` is the index of q0;
/// samples are at base ± i*step. p_i = plane[base-(i+1)step], q_i=plane[base+i*step].
#[allow(clippy::too_many_arguments)]
fn filter_line(plane: &mut [u8], base: usize, step: usize, bs: u8, alpha: i32, beta: i32, tc0: i32, chroma: bool) {
    let g = |o: isize| plane[(base as isize + o) as usize] as i32;
    let p0 = g(-(step as isize));
    let p1 = g(-2 * step as isize);
    let p2 = g(-3 * step as isize);
    let p3 = g(-4 * step as isize);
    let q0 = g(0);
    let q1 = g(step as isize);
    let q2 = g(2 * step as isize);
    let q3 = g(3 * step as isize);
    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }
    let cl = |v: i32| v.clamp(0, 255) as u8;
    let set = |plane: &mut [u8], o: isize, v: u8| plane[(base as isize + o) as usize] = v;

    if bs < 4 {
        let ap = (p2 - p0).abs();
        let aq = (q2 - q0).abs();
        let mut tc = tc0;
        if !chroma {
            tc += (ap < beta) as i32 + (aq < beta) as i32;
        } else {
            tc += 1;
        }
        let delta = (((q0 - p0) * 4 + (p1 - q1) + 4) >> 3).clamp(-tc, tc);
        set(plane, -(step as isize), cl(p0 + delta));
        set(plane, 0, cl(q0 - delta));
        if !chroma {
            if ap < beta {
                let d = ((p2 + ((p0 + q0 + 1) >> 1) - 2 * p1) >> 1).clamp(-tc0, tc0);
                set(plane, -2 * step as isize, cl(p1 + d));
            }
            if aq < beta {
                let d = ((q2 + ((p0 + q0 + 1) >> 1) - 2 * q1) >> 1).clamp(-tc0, tc0);
                set(plane, step as isize, cl(q1 + d));
            }
        }
    } else {
        // bS == 4 (strong, intra MB boundary).
        let small = (p0 - q0).abs() < ((alpha >> 2) + 2);
        if chroma {
            set(plane, -(step as isize), cl((2 * p1 + p0 + q1 + 2) >> 2));
            set(plane, 0, cl((2 * q1 + q0 + p1 + 2) >> 2));
        } else {
            let ap = (p2 - p0).abs();
            let aq = (q2 - q0).abs();
            if ap < beta && small {
                set(plane, -(step as isize), cl((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3));
                set(plane, -2 * step as isize, cl((p2 + p1 + p0 + q0 + 2) >> 2));
                set(plane, -3 * step as isize, cl((2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3));
            } else {
                set(plane, -(step as isize), cl((2 * p1 + p0 + q1 + 2) >> 2));
            }
            if aq < beta && small {
                set(plane, 0, cl((q2 + 2 * q1 + 2 * q0 + 2 * p0 + p1 + 4) >> 3));
                set(plane, step as isize, cl((q2 + q1 + q0 + p0 + 2) >> 2));
                set(plane, 2 * step as isize, cl((2 * q3 + 3 * q2 + q1 + q0 + p0 + 4) >> 3));
            } else {
                set(plane, 0, cl((2 * q1 + q0 + p1 + 2) >> 2));
            }
        }
    }
}

// ===== prediction, motion compensation, deblocking, output =====================

impl Decoder {
    fn mark_decoded(&mut self, mbx: usize, mby: usize, bx: usize, by: usize, bw: usize, bh: usize) {
        let w4 = self.g4w;
        for j in 0..bh {
            for i in 0..bw {
                let (ax, ay) = (mbx * 4 + bx + i, mby * 4 + by + j);
                self.decoded4[ay * w4 + ax] = true;
            }
        }
    }

    fn set_mb_refidx(&mut self, mbx: usize, mby: usize, ref_idx: i8) {
        let w4 = self.g4w;
        for j in 0..4 {
            for i in 0..4 {
                let (ax, ay) = (mbx * 4 + i, mby * 4 + j);
                self.refidx[ay * w4 + ax] = ref_idx;
                self.mvx[ay * w4 + ax] = 0;
                self.mvy[ay * w4 + ax] = 0;
            }
        }
    }

    fn store_mb_inter(&mut self, mbx: usize, mby: usize, bx: usize, by: usize, bw: usize, bh: usize, mvx: i16, mvy: i16, ref_idx: i8) {
        let w4 = self.g4w;
        for j in 0..bh {
            for i in 0..bw {
                let (ax, ay) = (mbx * 4 + bx + i, mby * 4 + by + j);
                let idx = ay * w4 + ax;
                self.mvx[idx] = mvx;
                self.mvy[idx] = mvy;
                self.refidx[idx] = ref_idx;
                self.decoded4[idx] = true;
            }
        }
    }

    /// Fetch (mvx, mvy, ref, available) for the 4×4 block at absolute coords.
    fn nb_block(&self, ax: isize, ay: isize) -> (i32, i32, i32, bool) {
        let w4 = self.g4w as isize;
        let h4 = (self.h_mbs * 4) as isize;
        if ax < 0 || ay < 0 || ax >= w4 || ay >= h4 {
            return (0, 0, -1, false);
        }
        let idx = (ay * w4 + ax) as usize;
        if !self.decoded4[idx] {
            return (0, 0, -1, false);
        }
        (self.mvx[idx] as i32, self.mvy[idx] as i32, self.refidx[idx] as i32, true)
    }

    fn neighbor_mv(&self, mbx: usize, mby: usize, dx4: isize, dy4: isize) -> (i32, i32, i32) {
        let ax = mbx as isize * 4 + dx4;
        let ay = mby as isize * 4 + dy4;
        let (mx, my, r, _) = self.nb_block(ax, ay);
        (mx, my, r)
    }

    /// Median motion vector predictor (ITU §8.4.1.3) for a partition.
    fn predict_mv(&self, mbx: usize, mby: usize, bx: usize, by: usize, bw: usize, bh: usize, ref_idx: i32) -> (i16, i16) {
        let px = mbx as isize * 4 + bx as isize;
        let py = mby as isize * 4 + by as isize;
        let a = self.nb_block(px - 1, py); // left
        let b = self.nb_block(px, py - 1); // top
        let mut c = self.nb_block(px + bw as isize, py - 1); // top-right
        if !c.3 {
            c = self.nb_block(px - 1, py - 1); // top-left fallback (D)
        }

        // Directional shortcuts for 16×8 / 8×16.
        if bw == 4 && bh == 2 {
            if by == 0 && b.2 == ref_idx {
                return (b.0 as i16, b.1 as i16);
            }
            if by == 2 && a.2 == ref_idx {
                return (a.0 as i16, a.1 as i16);
            }
        } else if bw == 2 && bh == 4 {
            if bx == 0 && a.2 == ref_idx {
                return (a.0 as i16, a.1 as i16);
            }
            if bx == 2 && c.2 == ref_idx {
                return (c.0 as i16, c.1 as i16);
            }
        }

        // If B and C are both unavailable but A is available, use A.
        let (mut bb, mut cc) = (b, c);
        if !b.3 && !c.3 && a.3 {
            bb = a;
            cc = a;
        }
        let match_a = a.2 == ref_idx;
        let match_b = bb.2 == ref_idx;
        let match_c = cc.2 == ref_idx;
        if match_a as i32 + match_b as i32 + match_c as i32 == 1 {
            let m = if match_a { a } else if match_b { bb } else { cc };
            return (m.0 as i16, m.1 as i16);
        }
        (
            median(a.0, bb.0, cc.0) as i16,
            median(a.1, bb.1, cc.1) as i16,
        )
    }

    // ---- intra 4×4 -----------------------------------------------------------

    fn pred_i4_mode(&self, mbx: usize, mby: usize, bx: usize, by: usize) -> i8 {
        let w4 = self.g4w;
        let ax = mbx * 4 + bx;
        let ay = mby * 4 + by;
        // Availability for *mode* prediction tracks whether the neighbour's
        // intra4x4 mode is known (i4set), not whether it's reconstructed —
        // intra-MB neighbours are read earlier in scan order.
        let left_avail = ax > 0 && self.i4set[ay * w4 + ax - 1];
        let top_avail = ay > 0 && self.i4set[(ay - 1) * w4 + ax];
        if !left_avail || !top_avail {
            return 2; // DC
        }
        let ml = self.i4mode[ay * w4 + ax - 1];
        let mt = self.i4mode[(ay - 1) * w4 + ax];
        ml.min(mt)
    }

    fn intra4x4_predict(&mut self, mbx: usize, mby: usize, bx: usize, by: usize, mode: u32) {
        let ys = self.w_mbs * 16;
        let yh = self.h_mbs * 16;
        let ox = mbx * 16 + bx * 4;
        let oy = mby * 16 + by * 4;
        let w4 = self.g4w;
        let ax = mbx * 4 + bx;
        let ay = mby * 4 + by;
        let left_avail = ox > 0;
        let top_avail = oy > 0;
        let tl_avail = left_avail && top_avail;
        // top-right availability via decode grid.
        let tr_avail = top_avail
            && (ax + 1) < w4
            && self.decoded4[(ay - 1) * w4 + ax + 1];

        let g = |x: isize, y: isize, plane: &[u8]| -> i32 {
            let xx = (ox as isize + x).clamp(0, ys as isize - 1) as usize;
            let yy = (oy as isize + y).clamp(0, yh as isize - 1) as usize;
            plane[yy * ys + xx] as i32
        };
        let mut t = [0i32; 8];
        let mut l = [0i32; 4];
        for i in 0..4 {
            t[i] = if top_avail { g(i as isize, -1, &self.y) } else { 128 };
            l[i] = if left_avail { g(-1, i as isize, &self.y) } else { 128 };
        }
        for i in 4..8 {
            t[i] = if tr_avail {
                g(i as isize, -1, &self.y)
            } else {
                t[3]
            };
        }
        let tl = if tl_avail { g(-1, -1, &self.y) } else { 128 };
        // pxy: x,y in -1..3 referencing reference samples.
        let pxy = |x: isize, y: isize| -> i32 {
            if x == -1 && y == -1 {
                tl
            } else if y == -1 {
                t[x as usize]
            } else {
                l[y as usize]
            }
        };

        let mut pred = [[0i32; 4]; 4];
        match mode {
            0 => {
                for y in 0..4 {
                    for x in 0..4 {
                        pred[y][x] = t[x];
                    }
                }
            }
            1 => {
                for y in 0..4 {
                    for x in 0..4 {
                        pred[y][x] = l[y];
                    }
                }
            }
            2 => {
                let dc = if top_avail && left_avail {
                    (t[0] + t[1] + t[2] + t[3] + l[0] + l[1] + l[2] + l[3] + 4) >> 3
                } else if top_avail {
                    (t[0] + t[1] + t[2] + t[3] + 2) >> 2
                } else if left_avail {
                    (l[0] + l[1] + l[2] + l[3] + 2) >> 2
                } else {
                    128
                };
                for y in 0..4 {
                    for x in 0..4 {
                        pred[y][x] = dc;
                    }
                }
            }
            3 => {
                for y in 0..4usize {
                    for x in 0..4usize {
                        let s = x + y;
                        pred[y][x] = if x == 3 && y == 3 {
                            (t[6] + 3 * t[7] + 2) >> 2
                        } else {
                            (t[s] + 2 * t[s + 1] + t[s + 2] + 2) >> 2
                        };
                    }
                }
            }
            4 => {
                for y in 0..4isize {
                    for x in 0..4isize {
                        pred[y as usize][x as usize] = if x > y {
                            (pxy(x - y - 2, -1) + 2 * pxy(x - y - 1, -1) + pxy(x - y, -1) + 2) >> 2
                        } else if x < y {
                            (pxy(-1, y - x - 2) + 2 * pxy(-1, y - x - 1) + pxy(-1, y - x) + 2) >> 2
                        } else {
                            (t[0] + 2 * tl + l[0] + 2) >> 2
                        };
                    }
                }
            }
            5 => {
                for y in 0..4isize {
                    for x in 0..4isize {
                        let z = 2 * x - y;
                        pred[y as usize][x as usize] = if z >= 0 && z % 2 == 0 {
                            (pxy(x - (y >> 1) - 1, -1) + pxy(x - (y >> 1), -1) + 1) >> 1
                        } else if z >= 0 {
                            (pxy(x - (y >> 1) - 2, -1) + 2 * pxy(x - (y >> 1) - 1, -1) + pxy(x - (y >> 1), -1) + 2) >> 2
                        } else if z == -1 {
                            (l[0] + 2 * tl + t[0] + 2) >> 2
                        } else {
                            (pxy(-1, y - 1) + 2 * pxy(-1, y - 2) + pxy(-1, y - 3) + 2) >> 2
                        };
                    }
                }
            }
            6 => {
                for y in 0..4isize {
                    for x in 0..4isize {
                        let z = 2 * y - x;
                        pred[y as usize][x as usize] = if z >= 0 && z % 2 == 0 {
                            (pxy(-1, y - (x >> 1) - 1) + pxy(-1, y - (x >> 1)) + 1) >> 1
                        } else if z >= 0 {
                            (pxy(-1, y - (x >> 1) - 2) + 2 * pxy(-1, y - (x >> 1) - 1) + pxy(-1, y - (x >> 1)) + 2) >> 2
                        } else if z == -1 {
                            (l[0] + 2 * tl + t[0] + 2) >> 2
                        } else {
                            (pxy(x - 1, -1) + 2 * pxy(x - 2, -1) + pxy(x - 3, -1) + 2) >> 2
                        };
                    }
                }
            }
            7 => {
                for y in 0..4usize {
                    for x in 0..4usize {
                        let h = x + (y >> 1);
                        pred[y][x] = if y % 2 == 0 {
                            (t[h] + t[h + 1] + 1) >> 1
                        } else {
                            (t[h] + 2 * t[h + 1] + t[h + 2] + 2) >> 2
                        };
                    }
                }
            }
            _ => {
                // 8 Horizontal-Up
                for y in 0..4isize {
                    for x in 0..4isize {
                        let z = x + 2 * y;
                        let h = (y + (x >> 1)) as usize;
                        pred[y as usize][x as usize] = if z < 5 && z % 2 == 0 {
                            (l[h] + l[h + 1] + 1) >> 1
                        } else if z < 5 {
                            (l[h] + 2 * l[h + 1] + l[(h + 2).min(3)] + 2) >> 2
                        } else if z == 5 {
                            (l[2] + 3 * l[3] + 2) >> 2
                        } else {
                            l[3]
                        };
                    }
                }
            }
        }
        for y in 0..4 {
            for x in 0..4 {
                self.y[(oy + y) * ys + (ox + x)] = clip(pred[y][x]);
            }
        }
    }

    // ---- intra 16×16 ---------------------------------------------------------

    fn intra16x16_predict(&mut self, mbx: usize, mby: usize, mode: u32) {
        let ys = self.w_mbs * 16;
        let ox = mbx * 16;
        let oy = mby * 16;
        let top_avail = mby > 0;
        let left_avail = mbx > 0;
        let g = |x: isize, y: isize, plane: &[u8]| -> i32 {
            plane[((oy as isize + y) as usize) * ys + (ox as isize + x) as usize] as i32
        };
        let mut top = [0i32; 16];
        let mut left = [0i32; 16];
        for i in 0..16 {
            if top_avail {
                top[i] = g(i as isize, -1, &self.y);
            }
            if left_avail {
                left[i] = g(-1, i as isize, &self.y);
            }
        }
        let tl = if top_avail && left_avail { g(-1, -1, &self.y) } else { 0 };
        let mut out = [[0i32; 16]; 16];
        match mode {
            0 => {
                for y in 0..16 {
                    for x in 0..16 {
                        out[y][x] = top[x];
                    }
                }
            }
            1 => {
                for y in 0..16 {
                    for x in 0..16 {
                        out[y][x] = left[y];
                    }
                }
            }
            2 => {
                let dc = if top_avail && left_avail {
                    (top.iter().sum::<i32>() + left.iter().sum::<i32>() + 16) >> 5
                } else if top_avail {
                    (top.iter().sum::<i32>() + 8) >> 4
                } else if left_avail {
                    (left.iter().sum::<i32>() + 8) >> 4
                } else {
                    128
                };
                for y in 0..16 {
                    for x in 0..16 {
                        out[y][x] = dc;
                    }
                }
            }
            _ => {
                // plane
                let mut h = 0i32;
                let mut v = 0i32;
                for x in 0..8 {
                    let xp = x as i32;
                    h += (xp + 1) * (top[(8 + x).min(15)] - if 6 >= x { top[6 - x] } else { tl });
                }
                for y in 0..8 {
                    let yp = y as i32;
                    v += (yp + 1) * (left[(8 + y).min(15)] - if 6 >= y { left[6 - y] } else { tl });
                }
                let b = (5 * h + 32) >> 6;
                let c = (5 * v + 32) >> 6;
                let a = 16 * (left[15] + top[15]);
                for y in 0..16 {
                    for x in 0..16 {
                        out[y][x] = (a + b * (x as i32 - 7) + c * (y as i32 - 7) + 16) >> 5;
                    }
                }
            }
        }
        for y in 0..16 {
            for x in 0..16 {
                self.y[(oy + y) * ys + (ox + x)] = clip(out[y][x]);
            }
        }
    }

    // ---- intra chroma --------------------------------------------------------

    fn intra_chroma_predict(&mut self, mbx: usize, mby: usize, mode: u32) {
        for comp in 0..2 {
            self.intra_chroma_comp(comp, mbx, mby, mode);
        }
    }

    fn intra_chroma_comp(&mut self, comp: usize, mbx: usize, mby: usize, mode: u32) {
        let cs = self.w_mbs * 8;
        let ox = mbx * 8;
        let oy = mby * 8;
        let top_avail = mby > 0;
        let left_avail = mbx > 0;
        let plane: &mut Vec<u8> = if comp == 0 { &mut self.cb } else { &mut self.cr };
        let g = |x: isize, y: isize, plane: &Vec<u8>| -> i32 {
            plane[((oy as isize + y) as usize) * cs + (ox as isize + x) as usize] as i32
        };
        let mut top = [0i32; 8];
        let mut left = [0i32; 8];
        for i in 0..8 {
            if top_avail {
                top[i] = g(i as isize, -1, plane);
            }
            if left_avail {
                left[i] = g(-1, i as isize, plane);
            }
        }
        let tl = if top_avail && left_avail { g(-1, -1, plane) } else { 0 };
        let mut out = [[0i32; 8]; 8];
        match mode {
            0 => {
                // DC per 4×4 quadrant.
                for qy in 0..2 {
                    for qx in 0..2 {
                        let sa: i32 = (0..4).map(|i| top[qx * 4 + i]).sum();
                        let sl: i32 = (0..4).map(|i| left[qy * 4 + i]).sum();
                        let prefer_top = (qx, qy) == (1, 0);
                        let prefer_left = (qx, qy) == (0, 1);
                        let dc = if (qx == 0 && qy == 0) || (qx == 1 && qy == 1) {
                            if top_avail && left_avail {
                                (sa + sl + 4) >> 3
                            } else if top_avail {
                                (sa + 2) >> 2
                            } else if left_avail {
                                (sl + 2) >> 2
                            } else {
                                128
                            }
                        } else if prefer_top {
                            if top_avail {
                                (sa + 2) >> 2
                            } else if left_avail {
                                (sl + 2) >> 2
                            } else {
                                128
                            }
                        } else if prefer_left {
                            if left_avail {
                                (sl + 2) >> 2
                            } else if top_avail {
                                (sa + 2) >> 2
                            } else {
                                128
                            }
                        } else {
                            128
                        };
                        for y in 0..4 {
                            for x in 0..4 {
                                out[qy * 4 + y][qx * 4 + x] = dc;
                            }
                        }
                    }
                }
            }
            1 => {
                for y in 0..8 {
                    for x in 0..8 {
                        out[y][x] = left[y];
                    }
                }
            }
            2 => {
                for y in 0..8 {
                    for x in 0..8 {
                        out[y][x] = top[x];
                    }
                }
            }
            _ => {
                let mut h = 0i32;
                let mut v = 0i32;
                for x in 0..4 {
                    let xp = x as i32;
                    h += (xp + 1) * (top[(4 + x).min(7)] - if 2 >= x { top[2 - x] } else { tl });
                }
                for y in 0..4 {
                    let yp = y as i32;
                    v += (yp + 1) * (left[(4 + y).min(7)] - if 2 >= y { left[2 - y] } else { tl });
                }
                let b = (34 * h + 32) >> 6;
                let c = (34 * v + 32) >> 6;
                let a = 16 * (left[7] + top[7]);
                for y in 0..8 {
                    for x in 0..8 {
                        out[y][x] = (a + b * (x as i32 - 3) + c * (y as i32 - 3) + 16) >> 5;
                    }
                }
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                plane[(oy + y) * cs + (ox + x)] = clip(out[y][x]);
            }
        }
    }

    // ---- motion compensation -------------------------------------------------

    fn mc_luma(&mut self, mbx: usize, mby: usize, px_off: usize, py_off: usize, bw: usize, bh: usize, mvx: i16, mvy: i16) {
        if !self.has_ref {
            return;
        }
        let ys = self.w_mbs * 16;
        let yh = self.h_mbs * 16;
        let xf = (mvx & 3) as i32;
        let yf = (mvy & 3) as i32;
        for j in 0..bh {
            for i in 0..bw {
                let px = mbx * 16 + px_off + i;
                let py = mby * 16 + py_off + j;
                let ix = px as i32 + (mvx as i32 >> 2);
                let iy = py as i32 + (mvy as i32 >> 2);
                let v = luma_interp(&self.ref_y, ys, yh, ix, iy, xf, yf);
                self.y[py * ys + px] = v;
            }
        }
    }

    fn mc_chroma(&mut self, mbx: usize, mby: usize, px_off: usize, py_off: usize, bw: usize, bh: usize, mvx: i16, mvy: i16) {
        if !self.has_ref {
            return;
        }
        let cs = self.w_mbs * 8;
        let cht = self.h_mbs * 8;
        let xf = (mvx & 7) as i32;
        let yf = (mvy & 7) as i32;
        for comp in 0..2 {
            for j in 0..bh {
                for i in 0..bw {
                    let px = mbx * 8 + px_off + i;
                    let py = mby * 8 + py_off + j;
                    let ix = px as i32 + (mvx as i32 >> 3);
                    let iy = py as i32 + (mvy as i32 >> 3);
                    let refp = if comp == 0 { &self.ref_cb } else { &self.ref_cr };
                    let s = |dx: i32, dy: i32| -> i32 {
                        let xx = (ix + dx).clamp(0, cs as i32 - 1) as usize;
                        let yy = (iy + dy).clamp(0, cht as i32 - 1) as usize;
                        refp[yy * cs + xx] as i32
                    };
                    let a = s(0, 0);
                    let bb = s(1, 0);
                    let c = s(0, 1);
                    let d = s(1, 1);
                    let val = ((8 - xf) * (8 - yf) * a
                        + xf * (8 - yf) * bb
                        + (8 - xf) * yf * c
                        + xf * yf * d
                        + 32)
                        >> 6;
                    if comp == 0 {
                        self.cb[py * cs + px] = clip(val);
                    } else {
                        self.cr[py * cs + px] = clip(val);
                    }
                }
            }
        }
    }

    // ---- YCbCr -> RGB output (limited-range BT.601) ---------------------------

    fn to_rgb(&self) -> Frame {
        let ys = self.w_mbs * 16;
        let cs = self.w_mbs * 8;
        let (w, h) = (self.width, self.height);
        let mut pixels = vec![0u32; w * h];
        for y in 0..h {
            for x in 0..w {
                let yy = self.y[y * ys + x] as i32;
                let cb = self.cb[(y / 2) * cs + (x / 2)] as i32;
                let cr = self.cr[(y / 2) * cs + (x / 2)] as i32;
                let c = yy - 16;
                let d = cb - 128;
                let e = cr - 128;
                let r = clip((298 * c + 409 * e + 128) >> 8);
                let g = clip((298 * c - 100 * d - 208 * e + 128) >> 8);
                let bl = clip((298 * c + 516 * d + 128) >> 8);
                pixels[y * w + x] = 0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | bl as u32;
            }
        }
        Frame { w, h, pixels }
    }
}

/// Quarter-pel luma sample via 6-tap interpolation (ITU §8.4.2.2.1).
fn luma_interp(refp: &[u8], stride: usize, height: usize, ix: i32, iy: i32, xf: i32, yf: i32) -> u8 {
    let s = |dx: i32, dy: i32| -> i32 {
        let xx = (ix + dx).clamp(0, stride as i32 - 1) as usize;
        let yy = (iy + dy).clamp(0, height as i32 - 1) as usize;
        refp[yy * stride + xx] as i32
    };
    if xf == 0 && yf == 0 {
        return s(0, 0) as u8;
    }
    let tap = |a: i32, b: i32, c: i32, d: i32, e: i32, f: i32| a - 5 * b + 20 * c + 20 * d - 5 * e + f;
    // Horizontal half at a given row offset (unrounded).
    let hor = |dy: i32| tap(s(-2, dy), s(-1, dy), s(0, dy), s(1, dy), s(2, dy), s(3, dy));
    // Vertical half at a given col offset (unrounded).
    let ver = |dx: i32| tap(s(dx, -2), s(dx, -1), s(dx, 0), s(dx, 1), s(dx, 2), s(dx, 3));
    let cl = |v: i32| v.clamp(0, 255);
    let b = cl((hor(0) + 16) >> 5); // half horizontal at row 0
    let h = cl((ver(0) + 16) >> 5); // half vertical at col 0
    let s1 = cl((hor(1) + 16) >> 5); // half horizontal at row 1
    let m = cl((ver(1) + 16) >> 5); // half vertical at col 1
    let j = {
        let jj = tap(hor(-2), hor(-1), hor(0), hor(1), hor(2), hor(3));
        cl((jj + 512) >> 10)
    };
    let g0 = s(0, 0);
    let hh = s(1, 0);
    let mm = s(0, 1);
    let v = match (xf, yf) {
        (0, 0) => g0,
        (1, 0) => (g0 + b + 1) >> 1,
        (2, 0) => b,
        (3, 0) => (hh + b + 1) >> 1,
        (0, 1) => (g0 + h + 1) >> 1,
        (1, 1) => (b + h + 1) >> 1,
        (2, 1) => (b + j + 1) >> 1,
        (3, 1) => (b + m + 1) >> 1,
        (0, 2) => h,
        (1, 2) => (h + j + 1) >> 1,
        (2, 2) => j,
        (3, 2) => (j + m + 1) >> 1,
        (0, 3) => (mm + h + 1) >> 1,
        (1, 3) => (h + s1 + 1) >> 1,
        (2, 3) => (j + s1 + 1) >> 1,
        _ => (m + s1 + 1) >> 1,
    };
    cl(v) as u8
}
