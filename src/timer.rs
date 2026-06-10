//! ARM generic timer — the EL1 physical timer (CNTP_*_EL0; the EL2 boot
//! path grants EL1 access via CNTHCTL_EL2). Its INTID comes from the DTB's
//! armv8-timer node, not a hard-coded PPI number.

use crate::kprintln;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

const CTL_ENABLE: u64 = 1; // and IMASK (bit 1) left clear

static INTID: AtomicU32 = AtomicU32::new(0);
static RELOAD: AtomicU64 = AtomicU64::new(0);
static TICKS: AtomicU64 = AtomicU64::new(0);
static NEXT: AtomicU64 = AtomicU64::new(0); // absolute counter deadline
static QUIET: AtomicBool = AtomicBool::new(false);

/// Silence the per-tick serial line (the desktop uses the timer purely as
/// a periodic wakeup for its event loop).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn intid() -> u32 {
    INTID.load(Ordering::Relaxed)
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// --- wall clock (M19b) -------------------------------------------------------
// NTP anchors real UTC to the always-running hardware counter (cntpct),
// NOT the software tick: the tick rate changes between boot phases (10 Hz
// in M2, 50 Hz once the desktop starts) and the timer is even stopped
// across milestone12, so a tick anchor would skew. cntpct never stops, so
// wall time is exact regardless of what the periodic tick is doing.

static WALL_UNIX: AtomicU64 = AtomicU64::new(0); // UTC seconds at the anchor
static WALL_CNT: AtomicU64 = AtomicU64::new(0); // cntpct at the anchor
static WALL_TZ: AtomicI64 = AtomicI64::new(0); // local UTC offset, seconds
const WALL_HZ: u64 = 50; // sub-second resolution exported to the clock app

/// Record the real UTC time (NTP-derived) against the current counter.
pub fn set_wall(unix_seconds: u64) {
    WALL_CNT.store(counter(), Ordering::Relaxed);
    WALL_UNIX.store(unix_seconds, Ordering::Relaxed);
}

/// Set the local timezone offset (seconds east of UTC) read from TZ.TXT.
pub fn set_tz(offset_seconds: i64) {
    WALL_TZ.store(offset_seconds, Ordering::Relaxed);
}

pub fn tz_offset_seconds() -> i64 {
    WALL_TZ.load(Ordering::Relaxed)
}

/// True once NTP has anchored real time.
pub fn synced() -> bool {
    WALL_UNIX.load(Ordering::Relaxed) != 0
}

/// Local time as 50 Hz ticks since the unix epoch, if NTP ever synced.
/// Sub-second resolution so analog hands sweep. The timezone offset is
/// folded in so the clock app reads local wall time directly.
pub fn wall_ticks50() -> Option<u64> {
    let base = WALL_UNIX.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let elapsed = counter().wrapping_sub(WALL_CNT.load(Ordering::Relaxed));
    let sub = elapsed.saturating_mul(WALL_HZ) / frequency();
    let local = base as i64 + tz_offset_seconds();
    Some((local.max(0) as u64) * WALL_HZ + sub)
}

pub fn frequency() -> u64 {
    let freq: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)) };
    freq
}

fn counter() -> u64 {
    let now: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) now, options(nomem, nostack)) };
    now
}

fn set_cval(deadline: u64) {
    unsafe {
        core::arch::asm!("msr cntp_cval_el0, {}", in(reg) deadline, options(nomem, nostack))
    };
}

/// Program a periodic tick at `hz` and enable the timer. The caller still
/// has to enable the INTID at the GIC and unmask IRQs.
pub fn start(intid: u32, hz: u64) {
    let reload = frequency() / hz;
    INTID.store(intid, Ordering::Relaxed);
    RELOAD.store(reload, Ordering::Relaxed);
    let next = counter() + reload;
    NEXT.store(next, Ordering::Relaxed);
    set_cval(next);
    unsafe {
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) CTL_ENABLE, options(nomem, nostack))
    };
}

/// Disable the timer (the M2 demo is done; M11 re-enables it for preemption).
pub fn stop() {
    unsafe { core::arch::asm!("msr cntp_ctl_el0, xzr", options(nomem, nostack)) };
}

/// Called from the IRQ handler: rearm (which clears the level-triggered
/// interrupt condition) and count the tick.
///
/// Rearming is against an ABSOLUTE deadline (CVAL), not "reload from
/// now" (TVAL): TVAL restarts the period at handler-run time, so every
/// bit of IRQ latency stretches the tick and `ticks()` drifts behind
/// wall time — visibly slow clocks under TCG load (caught by M19). With
/// CVAL the cadence is anchored to the counter; if the handler fell a
/// whole period behind, the missed ticks are counted, not stretched.
pub fn on_tick() {
    let reload = RELOAD.load(Ordering::Relaxed);
    let now = counter();
    let mut next = NEXT.load(Ordering::Relaxed) + reload;
    let mut elapsed = 1;
    while next <= now {
        next += reload;
        elapsed += 1;
    }
    NEXT.store(next, Ordering::Relaxed);
    set_cval(next);
    let n = TICKS.fetch_add(elapsed, Ordering::Relaxed) + elapsed;
    if !QUIET.load(Ordering::Relaxed) {
        kprintln!("TICK: {n}");
    }
}
