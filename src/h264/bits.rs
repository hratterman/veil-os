//! RBSP bit reader with Exp-Golomb decoding for H.264 syntax elements.
//!
//! The caller passes an already-unescaped RBSP (emulation-prevention bytes
//! 0x03 removed). MSB-first bit order, as the spec mandates.

pub struct BitReader<'a> {
    data: &'a [u8],
    pub pos: usize, // bit position
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0 }
    }

    #[inline]
    pub fn bit(&mut self) -> u32 {
        let byte = *self.data.get(self.pos >> 3).unwrap_or(&0) as u32;
        let b = (byte >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        b
    }

    pub fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit();
        }
        v
    }

    /// ue(v): unsigned Exp-Golomb.
    pub fn ue(&mut self) -> u32 {
        let mut zeros = 0u32;
        while self.pos < self.data.len() * 8 && self.bit() == 0 {
            zeros += 1;
            if zeros > 31 {
                return 0;
            }
        }
        if zeros == 0 {
            return 0;
        }
        let info = self.bits(zeros);
        (1u32 << zeros) - 1 + info
    }

    /// se(v): signed Exp-Golomb.
    pub fn se(&mut self) -> i32 {
        let k = self.ue();
        let sign = if k & 1 != 0 { 1 } else { -1 };
        sign * ((k + 1) / 2) as i32
    }

    /// te(v): truncated Exp-Golomb with given range (used for ref_idx etc.).
    /// For range==1 it's a single inverted bit; otherwise ue(v).
    pub fn te(&mut self, range: u32) -> u32 {
        if range == 1 {
            1 - self.bit()
        } else {
            self.ue()
        }
    }

    /// Are there more RBSP data bits before the trailing stop bit?
    pub fn more_rbsp_data(&self) -> bool {
        let total = self.data.len() * 8;
        if self.pos >= total {
            return false;
        }
        // Find the last set bit (the rbsp_stop_one_bit). If our position is
        // before it, there is more data.
        let mut last = total;
        while last > 0 {
            last -= 1;
            let byte = self.data[last >> 3];
            if (byte >> (7 - (last & 7))) & 1 == 1 {
                break;
            }
        }
        self.pos < last
    }

    pub fn byte_aligned(&self) -> bool {
        self.pos & 7 == 0
    }
}

/// Strip emulation-prevention bytes (00 00 03 -> 00 00) from a NAL payload,
/// producing the RBSP.
pub fn unescape(nal: &[u8]) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(nal.len());
    let mut zeros = 0u32;
    let mut i = 0;
    while i < nal.len() {
        let b = nal[i];
        if zeros >= 2 && b == 0x03 && i + 1 < nal.len() && nal[i + 1] <= 0x03 {
            // Skip the emulation-prevention 0x03.
            zeros = 0;
            i += 1;
            continue;
        }
        if b == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
        out.push(b);
        i += 1;
    }
    out
}
