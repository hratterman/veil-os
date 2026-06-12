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

/// sin/cos for `no_std` via range reduction to [-pi, pi] + a Taylor polynomial.
pub fn sin(x: f64) -> f64 {
    taylor_sin(reduce(x))
}
pub fn cos(x: f64) -> f64 {
    taylor_sin(reduce(x + core::f64::consts::FRAC_PI_2))
}

fn reduce(x: f64) -> f64 {
    let tau = 2.0 * core::f64::consts::PI;
    let mut x = x % tau;
    if x > core::f64::consts::PI {
        x -= tau;
    }
    if x < -core::f64::consts::PI {
        x += tau;
    }
    x
}

fn taylor_sin(x: f64) -> f64 {
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))))
}
