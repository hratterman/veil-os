//! f64 rounding/sqrt for `no_std` (core lacks these methods). AArch64 has
//! single-instruction forms: frintm (floor), frintp (ceil), frintz (trunc),
//! frinta (round-to-nearest), fsqrt.

#[inline]
pub fn floor(x: f64) -> f64 {
    let r: f64;
    unsafe { core::arch::asm!("frintm {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    r
}

#[inline]
pub fn ceil(x: f64) -> f64 {
    let r: f64;
    unsafe { core::arch::asm!("frintp {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    r
}

#[inline]
pub fn trunc(x: f64) -> f64 {
    let r: f64;
    unsafe { core::arch::asm!("frintz {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    r
}

#[inline]
pub fn sqrt(x: f64) -> f64 {
    let r: f64;
    unsafe { core::arch::asm!("fsqrt {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    r
}
