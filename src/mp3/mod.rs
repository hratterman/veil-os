//! From-scratch MPEG-1 Audio Layer III (MP3) decoder — no crates.
//!
//! Ported to Rust from the public-domain pdmp3.c reference (Unlicense; see
//! `vendor/pdmp3.c`), restructured to decode a whole in-memory file at once
//! (no streaming ring buffer) and to use precomputed tables instead of libm
//! (the kernel has no `sin`/`cos`/`pow`). The pipeline is the canonical Layer
//! III chain: frame sync + header, side info, scalefactors, Huffman decode,
//! requantization, reordering, stereo (M/S + intensity), alias reduction,
//! IMDCT + windowing (hybrid synthesis), frequency inversion, and the
//! polyphase synthesis filterbank → 16-bit PCM. Output feeds `snd::play`.

mod tables;
use tables as t;

use alloc::vec;
use alloc::vec::Vec;

/// Decoded PCM: interleaved 16-bit samples, always 2 channels (mono is
/// duplicated) so it drops straight into the virtio-sound stereo path.
pub struct Pcm {
    pub rate: u32,
    pub channels: u8,
    pub samples: Vec<i16>, // interleaved L,R,L,R,...
}

const SQRT2_INV: f32 = 0.707_106_77;

#[derive(Default, Clone, Copy)]
struct Header {
    layer: u32,            // 1,2,3 (we only decode 3)
    protection_bit: u32,   // 1 => no CRC
    bitrate_index: u32,
    sampling_frequency: u32, // 0=44100 1=48000 2=32000
    padding_bit: u32,
    mode: u32,             // 0=stereo 1=joint 2=dual 3=mono
    mode_extension: u32,
}

#[derive(Default)]
struct SideInfo {
    main_data_begin: u32,
    scfsi: [[u32; 4]; 2],
    part2_3_length: [[u32; 2]; 2],
    big_values: [[u32; 2]; 2],
    global_gain: [[u32; 2]; 2],
    scalefac_compress: [[u32; 2]; 2],
    win_switch_flag: [[u32; 2]; 2],
    block_type: [[u32; 2]; 2],
    mixed_block_flag: [[u32; 2]; 2],
    table_select: [[[u32; 3]; 2]; 2],
    subblock_gain: [[[u32; 3]; 2]; 2],
    region0_count: [[u32; 2]; 2],
    region1_count: [[u32; 2]; 2],
    preflag: [[u32; 2]; 2],
    scalefac_scale: [[u32; 2]; 2],
    count1table_select: [[u32; 2]; 2],
    count1: [[u32; 2]; 2],
}

struct MainData {
    scalefac_l: [[[u32; 21]; 2]; 2],
    scalefac_s: [[[[u32; 3]; 12]; 2]; 2],
    is: [[[f32; 576]; 2]; 2],
}

impl Default for MainData {
    fn default() -> MainData {
        MainData {
            scalefac_l: [[[0; 21]; 2]; 2],
            scalefac_s: [[[[0; 3]; 12]; 2]; 2],
            is: [[[0.0; 576]; 2]; 2],
        }
    }
}

/// A big-endian MSB-first bit reader over a byte slice, addressed by bit index.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize, // bit position
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0 }
    }
    fn bit(&mut self) -> u32 {
        let byte = *self.data.get(self.pos >> 3).unwrap_or(&0) as u32;
        let b = (byte >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        b
    }
    fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit();
        }
        v
    }
}

/// Persistent decoder state across frames: the bit reservoir and the two
/// FIFO/overlap buffers that the IMDCT and synthesis filterbank carry forward.
struct Decoder {
    header: Header,
    si: SideInfo,
    md: MainData,
    reservoir: [u8; 2048],
    res_top: usize,
    hsynth_store: [[[f32; 18]; 32]; 2], // overlap-add memory
    v_vec: [[f32; 1024]; 2],            // synthesis FIFO
    hsynth_init: bool,
    synth_init: bool,
}

impl Decoder {
    fn new() -> Decoder {
        Decoder {
            header: Header::default(),
            si: SideInfo::default(),
            md: MainData::default(),
            reservoir: [0; 2048],
            res_top: 0,
            hsynth_store: [[[0.0; 18]; 32]; 2],
            v_vec: [[0.0; 1024]; 2],
            hsynth_init: true,
            synth_init: true,
        }
    }

    fn nch(&self) -> usize {
        if self.header.mode == 3 { 1 } else { 2 }
    }
}

/// Decode a whole MP3 file to interleaved stereo PCM. Returns None if no valid
/// Layer III frame is found.
pub fn decode(data: &[u8]) -> Option<Pcm> {
    // Skip an ID3v2 tag if present.
    let mut p = 0usize;
    if data.len() > 10 && &data[0..3] == b"ID3" {
        let sz = ((data[6] as usize & 0x7f) << 21)
            | ((data[7] as usize & 0x7f) << 14)
            | ((data[8] as usize & 0x7f) << 7)
            | (data[9] as usize & 0x7f);
        p = 10 + sz;
    }

    let mut dec = Decoder::new();
    let mut out: Vec<i16> = Vec::new();
    let mut rate = 0u32;
    let mut nch_final = 2u8;
    let mut frames = 0usize;

    while p + 4 <= data.len() {
        // Find frame sync 0xFFE.
        if data[p] != 0xFF || (data[p + 1] & 0xE0) != 0xE0 {
            p += 1;
            continue;
        }
        let Some(h) = parse_header(&data[p..]) else {
            p += 1;
            continue;
        };
        // Only MPEG-1 Layer III with valid indices.
        if h.layer != 3 || h.bitrate_index == 0 || h.bitrate_index == 15 || h.sampling_frequency == 3 {
            p += 1;
            continue;
        }
        let framesize = frame_size(&h);
        if framesize < 24 || p + framesize > data.len() + 1 {
            p += 1;
            continue;
        }
        dec.header = h;
        rate = t::SAMPLE_RATES[h.sampling_frequency as usize];
        nch_final = dec.nch() as u8;

        let frame = &data[p..(p + framesize).min(data.len())];
        if let Some(()) = dec.decode_frame(frame, &mut out) {
            frames += 1;
        }
        p += framesize;
    }

    if frames == 0 {
        return None;
    }
    Some(Pcm { rate, channels: nch_final, samples: out })
}

fn parse_header(d: &[u8]) -> Option<Header> {
    if d.len() < 4 {
        return None;
    }
    let h = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
    if (h >> 21) & 0x7ff != 0x7ff {
        return None;
    }
    let id = (h >> 19) & 1; // MPEG-1 == 1 (bit pattern 11; we only read low bit of the 2)
    let version2 = (h >> 19) & 3;
    if version2 != 3 {
        return None; // require MPEG version 1 (11)
    }
    let _ = id;
    let layer_bits = (h >> 17) & 3;
    Some(Header {
        layer: 4 - layer_bits, // 01->3
        protection_bit: (h >> 16) & 1,
        bitrate_index: (h >> 12) & 0xf,
        sampling_frequency: (h >> 10) & 3,
        padding_bit: (h >> 9) & 1,
        mode: (h >> 6) & 3,
        mode_extension: (h >> 4) & 3,
    })
}

fn frame_size(h: &Header) -> usize {
    let br = t::BITRATES_L3[h.bitrate_index as usize];
    let sr = t::SAMPLE_RATES[h.sampling_frequency as usize];
    (144 * br / sr) as usize + h.padding_bit as usize
}

impl Decoder {
    /// Decode one frame `frame` (sync..end) and append its PCM to `out`.
    fn decode_frame(&mut self, frame: &[u8], out: &mut Vec<i16>) -> Option<()> {
        let nch = self.nch();
        let crc = if self.header.protection_bit == 0 { 2 } else { 0 };
        let sideinfo_size = if nch == 1 { 17 } else { 32 };
        let hdr_off = 4 + crc;
        if frame.len() < hdr_off + sideinfo_size {
            return None;
        }
        let side = &frame[hdr_off..hdr_off + sideinfo_size];
        self.read_side_info(side, nch);

        let main = &frame[hdr_off + sideinfo_size..];
        let main_size = main.len();
        let begin = self.si.main_data_begin as usize;

        // Assemble the main-data buffer from the reservoir (bytes borrowed from
        // previous frames) followed by this frame's main data.
        if begin > self.res_top {
            // Not enough history yet: stash this frame's bytes and skip decode.
            if self.res_top + main_size <= self.reservoir.len() {
                self.reservoir[self.res_top..self.res_top + main_size].copy_from_slice(main);
                self.res_top += main_size;
            } else {
                self.res_top = 0;
            }
            return None;
        }
        // Move the borrowed tail to the front, then append this frame's data.
        self.reservoir.copy_within(self.res_top - begin..self.res_top, 0);
        let total = begin + main_size;
        if total > self.reservoir.len() {
            self.res_top = 0;
            return None;
        }
        self.reservoir[begin..total].copy_from_slice(main);
        self.res_top = total;

        // Read scalefactors + Huffman from the assembled main data.
        let buf: Vec<u8> = self.reservoir[..total].to_vec();
        self.read_main(&buf, nch);

        // DSP: requantize, reorder, stereo, antialias, IMDCT, synthesis.
        let mut pcm = [[[0i16; 576]; 2]; 2]; // [gr][ch]
        for gr in 0..2 {
            for ch in 0..nch {
                self.requantize(gr, ch);
                self.reorder(gr, ch);
            }
            self.stereo(gr);
            for ch in 0..nch {
                self.antialias(gr, ch);
                self.hybrid_synthesis(gr, ch);
                self.frequency_inversion(gr, ch);
                let mut chout = [0i16; 576];
                self.subband_synthesis(gr, ch, &mut chout);
                pcm[gr][ch] = chout;
            }
        }
        // Interleave stereo (duplicate mono).
        for gr in 0..2 {
            for i in 0..576 {
                let l = pcm[gr][0][i];
                let r = if nch == 2 { pcm[gr][1][i] } else { l };
                out.push(l);
                out.push(r);
            }
        }
        Some(())
    }

    fn read_side_info(&mut self, side: &[u8], nch: usize) {
        let mut b = Bits::new(side);
        self.si = SideInfo::default();
        self.si.main_data_begin = b.bits(9);
        if nch == 1 {
            b.bits(5);
        } else {
            b.bits(3);
        }
        for ch in 0..nch {
            for band in 0..4 {
                self.si.scfsi[ch][band] = b.bit();
            }
        }
        for gr in 0..2 {
            for ch in 0..nch {
                self.si.part2_3_length[gr][ch] = b.bits(12);
                self.si.big_values[gr][ch] = b.bits(9);
                self.si.global_gain[gr][ch] = b.bits(8);
                self.si.scalefac_compress[gr][ch] = b.bits(4);
                self.si.win_switch_flag[gr][ch] = b.bit();
                if self.si.win_switch_flag[gr][ch] == 1 {
                    self.si.block_type[gr][ch] = b.bits(2);
                    self.si.mixed_block_flag[gr][ch] = b.bit();
                    for r in 0..2 {
                        self.si.table_select[gr][ch][r] = b.bits(5);
                    }
                    for w in 0..3 {
                        self.si.subblock_gain[gr][ch][w] = b.bits(3);
                    }
                    if self.si.block_type[gr][ch] == 2 && self.si.mixed_block_flag[gr][ch] == 0 {
                        self.si.region0_count[gr][ch] = 8;
                    } else {
                        self.si.region0_count[gr][ch] = 7;
                    }
                    self.si.region1_count[gr][ch] = 20 - self.si.region0_count[gr][ch];
                } else {
                    for r in 0..3 {
                        self.si.table_select[gr][ch][r] = b.bits(5);
                    }
                    self.si.region0_count[gr][ch] = b.bits(4);
                    self.si.region1_count[gr][ch] = b.bits(3);
                    self.si.block_type[gr][ch] = 0;
                }
                self.si.preflag[gr][ch] = b.bit();
                self.si.scalefac_scale[gr][ch] = b.bit();
                self.si.count1table_select[gr][ch] = b.bit();
            }
        }
    }

    fn read_main(&mut self, buf: &[u8], nch: usize) {
        let mut b = Bits::new(buf);
        self.md = MainData::default();
        for gr in 0..2 {
            for ch in 0..nch {
                let part_2_start = b.pos;
                let sc = self.si.scalefac_compress[gr][ch] as usize;
                let slen1 = t::SCALEFAC_SIZES[sc].0 as u32;
                let slen2 = t::SCALEFAC_SIZES[sc].1 as u32;
                if self.si.win_switch_flag[gr][ch] != 0 && self.si.block_type[gr][ch] == 2 {
                    if self.si.mixed_block_flag[gr][ch] != 0 {
                        for sfb in 0..8 {
                            self.md.scalefac_l[gr][ch][sfb] = b.bits(slen1);
                        }
                        for sfb in 3..12 {
                            let nbits = if sfb < 6 { slen1 } else { slen2 };
                            for win in 0..3 {
                                self.md.scalefac_s[gr][ch][sfb][win] = b.bits(nbits);
                            }
                        }
                    } else {
                        for sfb in 0..12 {
                            let nbits = if sfb < 6 { slen1 } else { slen2 };
                            for win in 0..3 {
                                self.md.scalefac_s[gr][ch][sfb][win] = b.bits(nbits);
                            }
                        }
                    }
                } else {
                    // Long blocks, with scfsi sharing from granule 0.
                    let ranges: [(usize, usize, u32); 4] =
                        [(0, 6, slen1), (6, 11, slen1), (11, 16, slen2), (16, 21, slen2)];
                    for (bandi, &(lo, hi, slen)) in ranges.iter().enumerate() {
                        if self.si.scfsi[ch][bandi] == 0 || gr == 0 {
                            for sfb in lo..hi {
                                self.md.scalefac_l[gr][ch][sfb] = b.bits(slen);
                            }
                        } else if gr == 1 {
                            for sfb in lo..hi {
                                self.md.scalefac_l[1][ch][sfb] = self.md.scalefac_l[0][ch][sfb];
                            }
                        }
                    }
                }
                self.read_huffman(&mut b, part_2_start, gr, ch);
            }
        }
    }

    fn read_huffman(&mut self, b: &mut Bits, part_2_start: usize, gr: usize, ch: usize) {
        let p23 = self.si.part2_3_length[gr][ch] as usize;
        if p23 == 0 {
            for i in 0..576 {
                self.md.is[gr][ch][i] = 0.0;
            }
            self.si.count1[gr][ch] = 0;
            return;
        }
        let bit_pos_end = part_2_start + p23 - 1;
        let sfreq = self.header.sampling_frequency as usize;
        let (region_1_start, region_2_start);
        if self.si.win_switch_flag[gr][ch] == 1 && self.si.block_type[gr][ch] == 2 {
            region_1_start = 36;
            region_2_start = 576;
        } else {
            let r0 = self.si.region0_count[gr][ch] as usize;
            let r1 = self.si.region1_count[gr][ch] as usize;
            region_1_start = t::SFB_LONG[sfreq][r0 + 1] as usize;
            region_2_start = t::SFB_LONG[sfreq][(r0 + r1 + 2).min(22)] as usize;
        }

        let bigvals = self.si.big_values[gr][ch] as usize * 2;
        let mut is_pos = 0usize;
        while is_pos < bigvals && is_pos < 576 {
            let table_num = if is_pos < region_1_start {
                self.si.table_select[gr][ch][0]
            } else if is_pos < region_2_start {
                self.si.table_select[gr][ch][1]
            } else {
                self.si.table_select[gr][ch][2]
            } as usize;
            let (x, y, _, _) = huffman_decode(b, table_num);
            self.md.is[gr][ch][is_pos] = x as f32;
            is_pos += 1;
            if is_pos < 576 {
                self.md.is[gr][ch][is_pos] = y as f32;
                is_pos += 1;
            }
        }

        // count1 region: quadruples.
        let table_num = (self.si.count1table_select[gr][ch] + 32) as usize;
        is_pos = bigvals;
        while is_pos <= 572 && b.pos <= bit_pos_end {
            let (x, y, v, w) = huffman_decode(b, table_num);
            self.md.is[gr][ch][is_pos] = v as f32;
            is_pos += 1;
            if is_pos >= 576 { break; }
            self.md.is[gr][ch][is_pos] = w as f32;
            is_pos += 1;
            if is_pos >= 576 { break; }
            self.md.is[gr][ch][is_pos] = x as f32;
            is_pos += 1;
            if is_pos >= 576 { break; }
            self.md.is[gr][ch][is_pos] = y as f32;
            is_pos += 1;
        }
        if b.pos > bit_pos_end + 1 && is_pos >= 4 {
            is_pos -= 4;
        }
        self.si.count1[gr][ch] = is_pos as u32;
        for i in is_pos..576 {
            self.md.is[gr][ch][i] = 0.0;
        }
        // Skip to the next part.
        b.pos = bit_pos_end + 1;
    }

    fn requantize(&mut self, gr: usize, ch: usize) {
        let sfreq = self.header.sampling_frequency as usize;
        let count1 = self.si.count1[gr][ch] as usize;
        let short_blocks =
            self.si.win_switch_flag[gr][ch] == 1 && self.si.block_type[gr][ch] == 2;
        if short_blocks {
            if self.si.mixed_block_flag[gr][ch] != 0 {
                // First 2 subbands (36 samples) are long.
                let mut sfb = 0usize;
                let mut next_sfb = t::SFB_LONG[sfreq][sfb + 1] as usize;
                for i in 0..36 {
                    if i == next_sfb {
                        sfb += 1;
                        next_sfb = t::SFB_LONG[sfreq][sfb + 1] as usize;
                    }
                    self.req_long(gr, ch, i, sfb);
                }
                self.req_short_loop(gr, ch, 36, count1, 3);
            } else {
                self.req_short_loop(gr, ch, 0, count1, 0);
            }
        } else {
            let mut sfb = 0usize;
            let mut next_sfb = t::SFB_LONG[sfreq][sfb + 1] as usize;
            for i in 0..count1 {
                if i == next_sfb {
                    sfb += 1;
                    next_sfb = t::SFB_LONG[sfreq][(sfb + 1).min(22)] as usize;
                }
                self.req_long(gr, ch, i, sfb);
            }
        }
    }

    fn req_short_loop(&mut self, gr: usize, ch: usize, start: usize, count1: usize, sfb0: usize) {
        let sfreq = self.header.sampling_frequency as usize;
        let mut sfb = sfb0;
        let mut next_sfb = t::SFB_SHORT[sfreq][sfb + 1] as usize * 3;
        let mut win_len =
            (t::SFB_SHORT[sfreq][sfb + 1] - t::SFB_SHORT[sfreq][sfb]) as usize;
        let mut i = start;
        while i < count1 {
            if i == next_sfb {
                sfb += 1;
                if sfb + 1 >= 14 {
                    break;
                }
                next_sfb = t::SFB_SHORT[sfreq][sfb + 1] as usize * 3;
                win_len = (t::SFB_SHORT[sfreq][sfb + 1] - t::SFB_SHORT[sfreq][sfb]) as usize;
            }
            for win in 0..3 {
                for _ in 0..win_len {
                    if i >= count1 {
                        break;
                    }
                    self.req_short(gr, ch, i, sfb, win);
                    i += 1;
                }
            }
        }
    }

    fn req_long(&mut self, gr: usize, ch: usize, is_pos: usize, sfb: usize) {
        let sf_mult = if self.si.scalefac_scale[gr][ch] != 0 { 2 } else { 1 }; // quarters: 1.0 or 0.5 -> *2 below
        let pf = self.si.preflag[gr][ch] as u32 * t::PRETAB[sfb.min(20)] as u32;
        let scalefac = self.md.scalefac_l[gr][ch][sfb.min(20)];
        // tmp1 = 2^(-(sf_mult_real * (scalefac+pf))) ; sf_mult_real = scalefac_scale?1:0.5
        // exponent in quarters: -(sf_mult * (scalefac+pf)) * 2  (since 0.5 -> 2 quarters, 1.0 -> 4)
        let e1_q = -((sf_mult * 2) as i32) * (scalefac + pf) as i32; // quarters
        // tmp2 = 2^(0.25*(global_gain-210)) -> quarters = (gg-210)
        let e2_q = self.si.global_gain[gr][ch] as i32 - 210;
        let scale = pow2_quarter(e1_q + e2_q);
        let v = self.md.is[gr][ch][is_pos];
        self.md.is[gr][ch][is_pos] = scale * pow43(v);
    }

    fn req_short(&mut self, gr: usize, ch: usize, is_pos: usize, sfb: usize, win: usize) {
        let sf_mult = if self.si.scalefac_scale[gr][ch] != 0 { 2 } else { 1 };
        let scalefac = self.md.scalefac_s[gr][ch][sfb.min(11)][win];
        let e1_q = -((sf_mult * 2) as i32) * scalefac as i32;
        let e2_q = self.si.global_gain[gr][ch] as i32
            - 210
            - 8 * self.si.subblock_gain[gr][ch][win] as i32;
        let scale = pow2_quarter(e1_q + e2_q);
        let v = self.md.is[gr][ch][is_pos];
        self.md.is[gr][ch][is_pos] = scale * pow43(v);
    }

    fn reorder(&mut self, gr: usize, ch: usize) {
        if !(self.si.win_switch_flag[gr][ch] == 1 && self.si.block_type[gr][ch] == 2) {
            return;
        }
        let sfreq = self.header.sampling_frequency as usize;
        let mut re = [0.0f32; 576];
        let mut sfb = if self.si.mixed_block_flag[gr][ch] != 0 { 3 } else { 0 };
        let mut next_sfb = t::SFB_SHORT[sfreq][sfb + 1] as usize * 3;
        let mut win_len =
            (t::SFB_SHORT[sfreq][sfb + 1] - t::SFB_SHORT[sfreq][sfb]) as usize;
        let mut i = if sfb == 0 { 0 } else { 36 };
        while i < 576 {
            if i == next_sfb {
                let base = 3 * t::SFB_SHORT[sfreq][sfb] as usize;
                for j in 0..3 * win_len {
                    self.md.is[gr][ch][base + j] = re[j];
                }
                if i >= self.si.count1[gr][ch] as usize {
                    return;
                }
                sfb += 1;
                if sfb + 1 >= 14 {
                    return;
                }
                next_sfb = t::SFB_SHORT[sfreq][sfb + 1] as usize * 3;
                win_len = (t::SFB_SHORT[sfreq][sfb + 1] - t::SFB_SHORT[sfreq][sfb]) as usize;
            }
            for win in 0..3 {
                for j in 0..win_len {
                    if i >= 576 {
                        break;
                    }
                    re[j * 3 + win] = self.md.is[gr][ch][i];
                    i += 1;
                }
            }
        }
        let base = 3 * t::SFB_SHORT[sfreq][12] as usize;
        for j in 0..3 * win_len {
            if base + j < 576 {
                self.md.is[gr][ch][base + j] = re[j];
            }
        }
    }

    fn stereo(&mut self, gr: usize) {
        if self.header.mode != 1 || self.header.mode_extension == 0 {
            return;
        }
        // M/S stereo.
        if self.header.mode_extension & 0x2 != 0 {
            let c0 = self.si.count1[gr][0] as usize;
            let c1 = self.si.count1[gr][1] as usize;
            let max_pos = c0.max(c1);
            for i in 0..max_pos {
                let l = (self.md.is[gr][0][i] + self.md.is[gr][1][i]) * SQRT2_INV;
                let r = (self.md.is[gr][0][i] - self.md.is[gr][1][i]) * SQRT2_INV;
                self.md.is[gr][0][i] = l;
                self.md.is[gr][1][i] = r;
            }
        }
        // Intensity stereo.
        if self.header.mode_extension & 0x1 != 0 {
            self.stereo_intensity(gr);
        }
    }

    fn stereo_intensity(&mut self, gr: usize) {
        let sfreq = self.header.sampling_frequency as usize;
        let c1 = self.si.count1[gr][1] as usize;
        let short_blocks =
            self.si.win_switch_flag[gr][0] == 1 && self.si.block_type[gr][0] == 2;
        if short_blocks {
            let start_sfb = if self.si.mixed_block_flag[gr][0] != 0 {
                for sfb in 0..8 {
                    if t::SFB_LONG[sfreq][sfb] as usize >= c1 {
                        self.is_long(gr, sfb);
                    }
                }
                3
            } else {
                0
            };
            for sfb in start_sfb..12 {
                if t::SFB_SHORT[sfreq][sfb] as usize * 3 >= c1 {
                    self.is_short(gr, sfb);
                }
            }
        } else {
            for sfb in 0..21 {
                if t::SFB_LONG[sfreq][sfb] as usize >= c1 {
                    self.is_long(gr, sfb);
                }
            }
        }
    }

    fn is_long(&mut self, gr: usize, sfb: usize) {
        let is_pos = self.md.scalefac_l[gr][0][sfb.min(20)] as usize;
        if is_pos == 7 {
            return;
        }
        let sfreq = self.header.sampling_frequency as usize;
        let (rl, rr) = is_ratio(is_pos);
        let start = t::SFB_LONG[sfreq][sfb] as usize;
        let stop = t::SFB_LONG[sfreq][(sfb + 1).min(22)] as usize;
        for i in start..stop {
            let v = self.md.is[gr][0][i];
            self.md.is[gr][0][i] = rl * v;
            self.md.is[gr][1][i] = rr * v;
        }
    }

    fn is_short(&mut self, gr: usize, sfb: usize) {
        let sfreq = self.header.sampling_frequency as usize;
        let win_len = (t::SFB_SHORT[sfreq][sfb + 1] - t::SFB_SHORT[sfreq][sfb]) as usize;
        for win in 0..3 {
            let is_pos = self.md.scalefac_s[gr][0][sfb.min(11)][win] as usize;
            if is_pos == 7 {
                continue;
            }
            let (rl, rr) = is_ratio(is_pos);
            let start = t::SFB_SHORT[sfreq][sfb] as usize * 3 + win_len * win;
            for i in start..start + win_len {
                if i >= 576 {
                    break;
                }
                let v = self.md.is[gr][0][i];
                self.md.is[gr][0][i] = rl * v;
                self.md.is[gr][1][i] = rr * v;
            }
        }
    }

    fn antialias(&mut self, gr: usize, ch: usize) {
        if self.si.win_switch_flag[gr][ch] == 1
            && self.si.block_type[gr][ch] == 2
            && self.si.mixed_block_flag[gr][ch] == 0
        {
            return;
        }
        let sblim = if self.si.win_switch_flag[gr][ch] == 1
            && self.si.block_type[gr][ch] == 2
            && self.si.mixed_block_flag[gr][ch] == 1
        {
            2
        } else {
            32
        };
        for sb in 1..sblim {
            for i in 0..8 {
                let li = 18 * sb - 1 - i;
                let ui = 18 * sb + i;
                let lv = self.md.is[gr][ch][li];
                let uv = self.md.is[gr][ch][ui];
                self.md.is[gr][ch][li] = lv * t::ANTIALIAS_CS[i] - uv * t::ANTIALIAS_CA[i];
                self.md.is[gr][ch][ui] = uv * t::ANTIALIAS_CS[i] + lv * t::ANTIALIAS_CA[i];
            }
        }
    }

    fn hybrid_synthesis(&mut self, gr: usize, ch: usize) {
        if self.hsynth_init {
            self.hsynth_store = [[[0.0; 18]; 32]; 2];
            self.hsynth_init = false;
        }
        for sb in 0..32 {
            let bt = if self.si.win_switch_flag[gr][ch] == 1
                && self.si.mixed_block_flag[gr][ch] == 1
                && sb < 2
            {
                0
            } else {
                self.si.block_type[gr][ch]
            };
            let mut in_buf = [0.0f32; 18];
            for i in 0..18 {
                in_buf[i] = self.md.is[gr][ch][sb * 18 + i];
            }
            let mut raw = [0.0f32; 36];
            imdct_win(&in_buf, &mut raw, bt);
            for i in 0..18 {
                self.md.is[gr][ch][sb * 18 + i] = raw[i] + self.hsynth_store[ch][sb][i];
                self.hsynth_store[ch][sb][i] = raw[i + 18];
            }
        }
    }

    fn frequency_inversion(&mut self, gr: usize, ch: usize) {
        let mut sb = 1;
        while sb < 32 {
            let mut i = 1;
            while i < 18 {
                self.md.is[gr][ch][sb * 18 + i] = -self.md.is[gr][ch][sb * 18 + i];
                i += 2;
            }
            sb += 2;
        }
    }

    fn subband_synthesis(&mut self, gr: usize, ch: usize, out: &mut [i16; 576]) {
        if self.synth_init {
            self.v_vec = [[0.0; 1024]; 2];
            self.synth_init = false;
        }
        let mut u_vec = [0.0f32; 512];
        let mut s_vec = [0.0f32; 32];
        for ss in 0..18 {
            // Shift the V FIFO up by 64.
            let mut i = 1023;
            while i > 63 {
                self.v_vec[ch][i] = self.v_vec[ch][i - 64];
                i -= 1;
            }
            for i in 0..32 {
                s_vec[i] = self.md.is[gr][ch][i * 18 + ss];
            }
            for i in 0..64 {
                let mut sum = 0.0f32;
                for j in 0..32 {
                    sum += t::N_WIN[i][j] * s_vec[j];
                }
                self.v_vec[ch][i] = sum;
            }
            for i in 0..8 {
                for j in 0..32 {
                    u_vec[(i << 6) + j] = self.v_vec[ch][(i << 7) + j];
                    u_vec[(i << 6) + j + 32] = self.v_vec[ch][(i << 7) + j + 96];
                }
            }
            for i in 0..512 {
                u_vec[i] *= t::SYNTH_DTBL[i];
            }
            for i in 0..32 {
                let mut sum = 0.0f32;
                for j in 0..16 {
                    sum += u_vec[(j << 5) + i];
                }
                let mut samp = (sum * 32767.0) as i32;
                samp = samp.clamp(-32767, 32767);
                out[32 * ss + i] = samp as i16;
            }
        }
    }
}

/// Decode one Huffman code word; returns (x, y, v, w). Tables > 31 are the
/// count1 quadruple tables and use v,w,x,y; otherwise only x,y.
fn huffman_decode(b: &mut Bits, table_num: usize) -> (i32, i32, i32, i32) {
    let (off, treelen, linbits) = t::HUFF_MAIN[table_num];
    if treelen == 0 {
        return (0, 0, 0, 0);
    }
    let ht = &t::HUFF[off as usize..];
    let mut point = 0usize;
    let mut bitsleft = 32i32;
    let mut error = true;
    let mut x = 0i32;
    let mut y = 0i32;
    loop {
        if ht[point] & 0xff00 == 0 {
            error = false;
            x = ((ht[point] >> 4) & 0xf) as i32;
            y = (ht[point] & 0xf) as i32;
            break;
        }
        if b.bit() != 0 {
            while (ht[point] & 0xff) as usize >= 250 {
                point += (ht[point] & 0xff) as usize;
            }
            point += (ht[point] & 0xff) as usize;
        } else {
            while (ht[point] >> 8) as usize >= 250 {
                point += (ht[point] >> 8) as usize;
            }
            point += (ht[point] >> 8) as usize;
        }
        bitsleft -= 1;
        if bitsleft <= 0 || point >= treelen as usize {
            break;
        }
    }
    if error {
        return (0, 0, 0, 0);
    }
    let (mut v, mut w) = (0i32, 0i32);
    if table_num > 31 {
        v = (y >> 3) & 1;
        w = (y >> 2) & 1;
        x = (y >> 1) & 1;
        y &= 1;
        if v > 0 && b.bit() == 1 { v = -v; }
        if w > 0 && b.bit() == 1 { w = -w; }
        if x > 0 && b.bit() == 1 { x = -x; }
        if y > 0 && b.bit() == 1 { y = -y; }
    } else {
        if linbits > 0 && x == 15 {
            x += b.bits(linbits as u32) as i32;
        }
        if x > 0 && b.bit() == 1 { x = -x; }
        if linbits > 0 && y == 15 {
            y += b.bits(linbits as u32) as i32;
        }
        if y > 0 && b.bit() == 1 { y = -y; }
    }
    (x, y, v, w)
}

/// IMDCT + windowing for one subband (18 in, 36 out), per block type.
fn imdct_win(in_buf: &[f32; 18], out: &mut [f32; 36], block_type: u32) {
    for v in out.iter_mut() {
        *v = 0.0;
    }
    if block_type == 2 {
        for i in 0..3 {
            for p in 0..12 {
                let mut sum = 0.0f32;
                for m in 0..6 {
                    sum += in_buf[i + 3 * m] * t::COS_S[m][p];
                }
                out[6 * i + p + 6] += sum * t::IMDCT_WIN[2][p];
            }
        }
    } else {
        let bt = block_type as usize;
        for p in 0..36 {
            let mut sum = 0.0f32;
            for m in 0..18 {
                sum += in_buf[m] * t::COS_L[m][p];
            }
            out[p] = sum * t::IMDCT_WIN[bt][p];
        }
    }
}

/// is_pos^(4/3) via the precomputed table.
fn pow43(v: f32) -> f32 {
    let n = v.abs() as usize;
    let r = if n < t::POW43.len() { t::POW43[n] } else { 0.0 };
    if v < 0.0 {
        -r
    } else {
        r
    }
}

/// 2^(k/4) for integer k, via f64 exponent-bit math (no libm). All MP3 requant
/// exponents are integer multiples of 1/4.
fn pow2_quarter(k: i32) -> f32 {
    const Q: [f64; 4] = [
        1.0,
        1.189_207_115_002_721,  // 2^(1/4)
        1.414_213_562_373_095,  // 2^(1/2)
        1.681_792_830_507_429,  // 2^(3/4)
    ];
    let q = k.rem_euclid(4) as usize;
    let i = (k - q as i32) / 4; // integer power of two
    let two_i = pow2i(i);
    (Q[q] * two_i) as f32
}

/// 2^i for integer i, by setting the f64 exponent field. Clamped to range.
fn pow2i(i: i32) -> f64 {
    let i = i.clamp(-1022, 1023);
    f64::from_bits((((i + 1023) as u64) & 0x7ff) << 52)
}

fn is_ratio(is_pos: usize) -> (f32, f32) {
    if is_pos == 6 {
        (1.0, 0.0)
    } else {
        let r = t::IS_RATIOS[is_pos.min(5)];
        (r / (1.0 + r), 1.0 / (1.0 + r))
    }
}
