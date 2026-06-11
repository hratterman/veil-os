//! From-scratch H.264 *baseline / constrained-baseline* decoder — no crates.
//!
//! Pipeline: MP4 (ISO-BMFF) demux **or** Annex-B start codes → NAL units →
//! SPS/PPS (Exp-Golomb) → slice header → macroblock layer with CAVLC entropy
//! decode → intra (4×4 / 16×16 / chroma) and inter (P-slice motion
//! compensation, 6-tap luma + bilinear chroma) prediction → 4×4 integer
//! inverse transform (+ Hadamard DC) → in-loop deblocking filter → YCbCr→RGB.
//! Produces `Frame`s of XRGB8888 pixels for the video player.
//!
//! Scope is constrained baseline (no CABAC, no B-frames, single reference,
//! no weighted prediction), which is what the test content and most simple
//! H.264 files use.

mod bits;
mod cavlc;
mod cavlc_tables;
mod deblock_tables;
mod slice;
mod transform;

use alloc::vec::Vec;
use bits::{unescape, BitReader};

/// A decoded frame: XRGB8888 pixels, row-major, w*h.
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<u32>,
}

#[derive(Default, Clone)]
pub struct Sps {
    pub profile_idc: u32,
    pub log2_max_frame_num: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_poc_lsb: u32,
    pub max_num_ref_frames: u32,
    pub pic_width_mbs: usize,
    pub pic_height_map_units: usize,
    pub frame_mbs_only: bool,
    pub direct_8x8: bool,
    pub crop_l: usize,
    pub crop_r: usize,
    pub crop_t: usize,
    pub crop_b: usize,
}

impl Sps {
    pub fn width(&self) -> usize {
        self.pic_width_mbs * 16 - (self.crop_l + self.crop_r) * 2
    }
    pub fn height(&self) -> usize {
        self.pic_height_map_units * 16 - (self.crop_t + self.crop_b) * 2
    }
    pub fn width_mbs(&self) -> usize {
        self.pic_width_mbs
    }
    pub fn height_mbs(&self) -> usize {
        self.pic_height_map_units
    }
}

#[derive(Default, Clone)]
pub struct Pps {
    pub entropy_coding_mode: u32, // 0 == CAVLC (baseline)
    pub num_ref_idx_l0_default: u32,
    pub weighted_pred: u32,
    pub pic_init_qp: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present: bool,
    pub constrained_intra_pred: bool,
    pub pic_order_present: bool,
}

fn parse_sps(rbsp: &[u8]) -> Sps {
    let mut b = BitReader::new(rbsp);
    let mut s = Sps::default();
    s.profile_idc = b.bits(8);
    b.bits(8); // constraint flags + reserved
    b.bits(8); // level_idc
    b.ue(); // seq_parameter_set_id
    // High-profile chroma fields (baseline won't hit these, but be safe).
    if matches!(s.profile_idc, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma_format = b.ue();
        if chroma_format == 3 {
            b.bit();
        }
        b.ue(); // bit_depth_luma
        b.ue(); // bit_depth_chroma
        b.bit(); // qpprime
        if b.bit() == 1 {
            // seq_scaling_matrix: skip (unused in baseline)
            let n = if chroma_format != 3 { 8 } else { 12 };
            for _ in 0..n {
                if b.bit() == 1 {
                    // we don't support custom scaling lists; bail-ish
                }
            }
        }
    }
    s.log2_max_frame_num = b.ue() + 4;
    s.pic_order_cnt_type = b.ue();
    if s.pic_order_cnt_type == 0 {
        s.log2_max_poc_lsb = b.ue() + 4;
    } else if s.pic_order_cnt_type == 1 {
        b.bit();
        b.se();
        b.se();
        let n = b.ue();
        for _ in 0..n {
            b.se();
        }
    }
    s.max_num_ref_frames = b.ue();
    b.bit(); // gaps_in_frame_num_value_allowed
    s.pic_width_mbs = (b.ue() + 1) as usize;
    s.pic_height_map_units = (b.ue() + 1) as usize;
    s.frame_mbs_only = b.bit() == 1;
    if !s.frame_mbs_only {
        b.bit(); // mb_adaptive_frame_field
    }
    s.direct_8x8 = b.bit() == 1;
    if b.bit() == 1 {
        // frame_cropping
        s.crop_l = b.ue() as usize;
        s.crop_r = b.ue() as usize;
        s.crop_t = b.ue() as usize;
        s.crop_b = b.ue() as usize;
    }
    // vui_parameters: ignored
    s
}

fn parse_pps(rbsp: &[u8]) -> Pps {
    let mut b = BitReader::new(rbsp);
    let mut p = Pps::default();
    b.ue(); // pps_id
    b.ue(); // sps_id
    p.entropy_coding_mode = b.bit();
    p.pic_order_present = b.bit() == 1;
    let num_slice_groups = b.ue() + 1;
    if num_slice_groups > 1 {
        // FMO: not supported in baseline content we target; parse-skip minimally.
        let map_type = b.ue();
        if map_type == 0 {
            for _ in 0..num_slice_groups {
                b.ue();
            }
        }
    }
    p.num_ref_idx_l0_default = b.ue() + 1;
    b.ue(); // num_ref_idx_l1_default
    p.weighted_pred = b.bit();
    b.bits(2); // weighted_bipred_idc
    p.pic_init_qp = 26 + b.se();
    b.se(); // pic_init_qs
    p.chroma_qp_index_offset = b.se();
    p.deblocking_filter_control_present = b.bit() == 1;
    p.constrained_intra_pred = b.bit() == 1;
    b.bit(); // redundant_pic_cnt_present
    p
}

/// Iterate NAL units of an Annex-B stream, calling `f(nal_ref_idc, type, payload)`.
fn for_each_annexb(data: &[u8], mut f: impl FnMut(u8, u8, &[u8])) {
    let mut i = 0;
    // Find first start code.
    let starts = find_start_codes(data);
    for (k, &(s, hdr)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|x| x.0).unwrap_or(data.len());
        let _ = i;
        i = s;
        if hdr >= end {
            continue;
        }
        let nal_hdr = data[hdr];
        let ref_idc = (nal_hdr >> 5) & 3;
        let ty = nal_hdr & 0x1f;
        f(ref_idc, ty, &data[hdr + 1..end]);
    }
}

/// Returns (start_of_startcode, index_of_nal_header_byte) for each NAL.
fn find_start_codes(data: &[u8]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            v.push((i, i + 3));
            i += 3;
        } else if i + 4 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            v.push((i, i + 4));
            i += 4;
        } else {
            i += 1;
        }
    }
    v
}

// ---- MP4 (ISO-BMFF) demux ----------------------------------------------------

fn be32(d: &[u8], i: usize) -> usize {
    ((d[i] as usize) << 24) | ((d[i + 1] as usize) << 16) | ((d[i + 2] as usize) << 8) | d[i + 3] as usize
}

/// Walk MP4 boxes to find `avcC` (parameter sets + nal length size) and the
/// `mdat` payload. Returns (sps_list, pps_list, nal_length_size, mdat_range).
struct Mp4 {
    sps: Vec<Vec<u8>>,
    pps: Vec<Vec<u8>>,
    nal_len_size: usize,
    samples: Vec<(usize, usize)>, // (offset, size) of each sample in the file
}

fn find_box<'a>(data: &'a [u8], mut start: usize, end: usize, name: &[u8; 4]) -> Option<(usize, usize)> {
    while start + 8 <= end {
        let size = be32(data, start);
        let bname = &data[start + 4..start + 8];
        let (body, boxend) = if size == 1 {
            // 64-bit size
            let large = be32(data, start + 8) * (1usize << 32) + be32(data, start + 12);
            (start + 16, start + large)
        } else if size == 0 {
            (start + 8, end)
        } else {
            (start + 8, start + size)
        };
        if bname == name {
            return Some((body, boxend.min(end)));
        }
        if boxend <= start {
            break;
        }
        start = boxend;
    }
    None
}

/// Recursively find a box path like ["moov","trak","mdia",...].
fn find_path(data: &[u8], range: (usize, usize), path: &[[u8; 4]]) -> Option<(usize, usize)> {
    let mut cur = range;
    for name in path {
        cur = find_box(data, cur.0, cur.1, name)?;
    }
    Some(cur)
}

fn parse_mp4(data: &[u8]) -> Option<Mp4> {
    let full = (0, data.len());
    let moov = find_box(data, 0, data.len(), b"moov")?;
    // Find the video trak that contains an avc1/avcC. Try each trak.
    let mut search = moov.0;
    let mut avcc_range = None;
    let mut stbl_range = None;
    while search < moov.1 {
        let Some(trak) = find_box(data, search, moov.1, b"trak") else { break };
        if let Some(stbl) = find_path(data, trak, &[*b"mdia", *b"minf", *b"stbl"]) {
            if let Some(stsd) = find_box(data, stbl.0, stbl.1, b"stsd") {
                // stsd: 8 bytes (version+count) then sample entries.
                if let Some(avcc) = find_box(data, stsd.0 + 8, stsd.1, b"avcC")
                    .or_else(|| find_avcc_in_stsd(data, stsd))
                {
                    avcc_range = Some(avcc);
                    stbl_range = Some(stbl);
                    break;
                }
            }
        }
        search = trak.1;
    }
    let avcc = avcc_range?;
    let stbl = stbl_range?;

    // Parse avcC: configurationVersion(1) profile(1) compat(1) level(1)
    // lengthSizeMinusOne(1, low2) numSPS(1, low5) then SPS list, then numPPS...
    let a = avcc.0;
    let nal_len_size = (data[a + 4] & 0x3) as usize + 1;
    let num_sps = (data[a + 5] & 0x1f) as usize;
    let mut o = a + 6;
    let mut sps = Vec::new();
    for _ in 0..num_sps {
        let len = ((data[o] as usize) << 8) | data[o + 1] as usize;
        o += 2;
        sps.push(data[o..o + len].to_vec());
        o += len;
    }
    let num_pps = data[o] as usize;
    o += 1;
    let mut pps = Vec::new();
    for _ in 0..num_pps {
        let len = ((data[o] as usize) << 8) | data[o + 1] as usize;
        o += 2;
        pps.push(data[o..o + len].to_vec());
        o += len;
    }

    // Sample table: stsz (sizes) + stco/co64 (chunk offsets) + stsc (samples
    // per chunk). For our simple files there's one sample per chunk or a flat
    // layout; build per-sample (offset, size).
    let samples = parse_sample_table(data, stbl).unwrap_or_default();
    let _ = full;
    Some(Mp4 { sps, pps, nal_len_size, samples })
}

fn find_avcc_in_stsd(data: &[u8], stsd: (usize, usize)) -> Option<(usize, usize)> {
    // The avc1 sample entry has a fixed 78-byte header before child boxes.
    let avc1 = find_box(data, stsd.0 + 8, stsd.1, b"avc1")
        .or_else(|| find_box(data, stsd.0 + 8, stsd.1, b"avc3"))?;
    find_box(data, avc1.0 + 78, avc1.1, b"avcC")
}

fn parse_sample_table(data: &[u8], stbl: (usize, usize)) -> Option<Vec<(usize, usize)>> {
    let stsz = find_box(data, stbl.0, stbl.1, b"stsz")?;
    let s = stsz.0;
    let sample_size = be32(data, s + 4);
    let count = be32(data, s + 8);
    let mut sizes = Vec::with_capacity(count);
    if sample_size != 0 {
        for _ in 0..count {
            sizes.push(sample_size);
        }
    } else {
        for i in 0..count {
            sizes.push(be32(data, s + 12 + i * 4));
        }
    }
    // Chunk offsets.
    let (offs, _is64) = if let Some(stco) = find_box(data, stbl.0, stbl.1, b"stco") {
        let c = be32(data, stco.0 + 4);
        let mut v = Vec::with_capacity(c);
        for i in 0..c {
            v.push(be32(data, stco.0 + 8 + i * 4));
        }
        (v, false)
    } else if let Some(co64) = find_box(data, stbl.0, stbl.1, b"co64") {
        let c = be32(data, co64.0 + 4);
        let mut v = Vec::with_capacity(c);
        for i in 0..c {
            v.push(be32(data, co64.0 + 8 + i * 8) * (1 << 32) + be32(data, co64.0 + 12 + i * 8));
        }
        (v, true)
    } else {
        return None;
    };
    // stsc: samples-per-chunk runs.
    let stsc = find_box(data, stbl.0, stbl.1, b"stsc")?;
    let runs_n = be32(data, stsc.0 + 4);
    let mut runs = Vec::with_capacity(runs_n); // (first_chunk, samples_per_chunk)
    for i in 0..runs_n {
        let first = be32(data, stsc.0 + 8 + i * 12);
        let spc = be32(data, stsc.0 + 8 + i * 12 + 4);
        runs.push((first, spc));
    }
    // Build per-sample offsets by walking chunks.
    let mut samples = Vec::with_capacity(count);
    let mut sample_idx = 0usize;
    for (ci, &chunk_off) in offs.iter().enumerate() {
        let chunk_num = ci + 1;
        // samples in this chunk from stsc
        let mut spc = 1;
        for &(first, s) in &runs {
            if chunk_num >= first {
                spc = s;
            }
        }
        let mut off = chunk_off;
        for _ in 0..spc {
            if sample_idx >= sizes.len() {
                break;
            }
            samples.push((off, sizes[sample_idx]));
            off += sizes[sample_idx];
            sample_idx += 1;
        }
    }
    Some(samples)
}

// ---- Top-level decode --------------------------------------------------------

/// Decode the whole stream into frames (in decode order). `data` may be MP4 or
/// raw Annex-B. Caps the frame count to bound memory.
pub fn decode_all(data: &[u8], max_frames: usize) -> Vec<Frame> {
    let mut dec = slice::Decoder::new();
    let mut frames = Vec::new();

    if data.len() > 8 && (&data[4..8] == b"ftyp" || &data[4..8] == b"moov") {
        if let Some(mp4) = parse_mp4(data) {
            for s in &mp4.sps {
                let r = unescape(&s[1..]);
                dec.set_sps(parse_sps(&r));
            }
            for p in &mp4.pps {
                let r = unescape(&p[1..]);
                dec.set_pps(parse_pps(&r));
            }
            for &(off, size) in &mp4.samples {
                if frames.len() >= max_frames {
                    break;
                }
                // Each sample is a sequence of length-prefixed NALs.
                let mut o = off;
                let end = (off + size).min(data.len());
                while o + mp4.nal_len_size <= end {
                    let mut len = 0usize;
                    for k in 0..mp4.nal_len_size {
                        len = (len << 8) | data[o + k] as usize;
                    }
                    o += mp4.nal_len_size;
                    if o + len > end {
                        break;
                    }
                    let nal = &data[o..o + len];
                    o += len;
                    let ref_idc = (nal[0] >> 5) & 3;
                    let ty = nal[0] & 0x1f;
                    let rbsp = unescape(&nal[1..]);
                    if let Some(f) = dec.handle_nal(ref_idc, ty, &rbsp) {
                        frames.push(f);
                    }
                }
            }
            return frames;
        }
    }

    // Annex-B path.
    let mut nals: Vec<(u8, u8, Vec<u8>)> = Vec::new();
    for_each_annexb(data, |ref_idc, ty, payload| {
        nals.push((ref_idc, ty, unescape(payload)));
    });
    for (ref_idc, ty, rbsp) in nals {
        if frames.len() >= max_frames {
            break;
        }
        if let Some(f) = dec.handle_nal(ref_idc, ty, &rbsp) {
            frames.push(f);
        }
    }
    frames
}

/// Probe just the SPS to get dimensions without decoding.
#[allow(dead_code)]
pub fn probe(data: &[u8]) -> Option<(usize, usize)> {
    if data.len() > 8 && (&data[4..8] == b"ftyp" || &data[4..8] == b"moov") {
        let mp4 = parse_mp4(data)?;
        let s = mp4.sps.first()?;
        let sps = parse_sps(&unescape(&s[1..]));
        return Some((sps.width(), sps.height()));
    }
    let mut dims = None;
    for_each_annexb(data, |_, ty, payload| {
        if ty == 7 && dims.is_none() {
            let sps = parse_sps(&unescape(payload));
            dims = Some((sps.width(), sps.height()));
        }
    });
    dims
}
