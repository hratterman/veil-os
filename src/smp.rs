//! M41 step 19: SMP — bring up the secondary CPU cores and run work on them.
//!
//! The primary core runs the OS + GUI; the secondary cores are **compute
//! workers**. They're started via PSCI `CPU_ON` (conduit read from the DTB),
//! each on its own stack, enable the shared kernel MMU, and spin in a worker
//! loop. `parallel_compute` forks a CPU-bound job across all online cores
//! (lock-free chunk dispatch via an atomic counter) and joins — measurably
//! faster than one core. `nproc()` reports the online count.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const MAX_CORES: usize = 4;

unsafe extern "C" {
    fn secondary_entry();
}

/// Per-core stack tops, read by `secondary_entry` (boot.s) before MMU is on.
#[unsafe(no_mangle)]
pub static mut SECONDARY_SP: [u64; MAX_CORES] = [0; MAX_CORES];

static ONLINE: [AtomicBool; MAX_CORES] = [
    AtomicBool::new(true), // core 0 is the primary
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];
static PSCI_HVC: AtomicBool = AtomicBool::new(false);

// --- a simple spinlock (for shared kernel data) -------------------------------

pub struct SpinLock {
    held: AtomicBool,
}

impl SpinLock {
    pub const fn new() -> SpinLock {
        SpinLock { held: AtomicBool::new(false) }
    }
    pub fn lock(&self) {
        while self.held.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }
    pub fn unlock(&self) {
        self.held.store(false, Ordering::Release);
    }
}

// --- fork/join parallel job ---------------------------------------------------

static GEN: AtomicU64 = AtomicU64::new(0);
static CHUNK: AtomicUsize = AtomicUsize::new(0);
static NCHUNKS: AtomicUsize = AtomicUsize::new(0);
static CHUNK_SZ: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static PARTIAL: [AtomicU64; MAX_CORES] = [
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
];

/// The CPU-bound kernel each chunk runs (a cheap, data-independent hash sum).
fn work(start: usize, end: usize) -> u64 {
    let mut acc = 0u64;
    for i in start..end {
        let x = i as u64;
        acc = acc.wrapping_add(x.wrapping_mul(x) ^ (x >> 3)).rotate_left(1);
    }
    acc
}

fn process_chunks(core: usize) {
    let n = NCHUNKS.load(Ordering::Relaxed);
    let sz = CHUNK_SZ.load(Ordering::Relaxed);
    loop {
        let c = CHUNK.fetch_add(1, Ordering::Relaxed);
        if c >= n {
            break;
        }
        let r = work(c * sz, (c + 1) * sz);
        PARTIAL[core].fetch_add(r, Ordering::Relaxed);
        DONE.fetch_add(1, Ordering::Release);
    }
}

/// Secondary core main, called from `secondary_entry` (asm). x0 = core index.
#[unsafe(no_mangle)]
extern "C" fn secondary_main(core: usize) -> ! {
    crate::paging::Mapper::enable_at(crate::scheduler::kernel_root());
    crate::exceptions::install();
    ONLINE[core.min(MAX_CORES - 1)].store(true, Ordering::SeqCst);
    crate::kprintln!("SMP: core {core} online");
    // Worker loop: wait for a job generation, process chunks, idle.
    let mut my_gen = GEN.load(Ordering::Acquire);
    loop {
        while GEN.load(Ordering::Acquire) == my_gen {
            unsafe { core::arch::asm!("wfe", options(nomem, nostack)) };
        }
        my_gen = GEN.load(Ordering::Acquire);
        process_chunks(core);
    }
}

/// Bring up the secondary cores via PSCI CPU_ON. Reads the conduit from the DTB.
pub fn bring_up(fdt: &crate::dtb::Fdt) {
    // PSCI conduit: "hvc" or "smc" from the /psci node's `method`.
    let hvc = fdt
        .find_compatible("arm,psci-0.2")
        .or_else(|| fdt.find_compatible("arm,psci-1.0"))
        .or_else(|| fdt.find_compatible("arm,psci"))
        .and_then(|n| fdt.prop(n, "method"))
        .map(|m| m.starts_with(b"hvc"))
        .unwrap_or(false);
    PSCI_HVC.store(hvc, Ordering::SeqCst);
    let entry = secondary_entry as usize as u64;

    for core in 1..MAX_CORES {
        // Allocate a 64 KiB stack for this core (identity-mapped RAM).
        let pages = 16;
        let Some(pa) = crate::frames::alloc_contiguous(pages) else { break };
        let top = (pa + pages * crate::frames::FRAME_SIZE) as u64;
        unsafe { core::ptr::addr_of_mut!(SECONDARY_SP).cast::<u64>().add(core).write(top) };
        // QEMU virt MPIDR: affinity level 0 == core index.
        let ret = psci_cpu_on(core as u64, entry, core as u64);
        if ret != 0 {
            crate::kprintln!("SMP: PSCI CPU_ON core {core} failed ({ret})");
        }
    }
    // Give the cores a moment to come online.
    for _ in 0..2_000_000 {
        core::hint::spin_loop();
    }
    crate::kprintln!("SMP: {} core(s) online (conduit={})", nproc(), if hvc { "hvc" } else { "smc" });
}

fn psci_cpu_on(target: u64, entry: u64, ctx: u64) -> i64 {
    const CPU_ON: u64 = 0xC400_0003; // PSCI_CPU_ON (SMC64)
    let ret: i64;
    unsafe {
        if PSCI_HVC.load(Ordering::SeqCst) {
            core::arch::asm!(
                "hvc #0",
                inout("x0") CPU_ON => ret,
                in("x1") target,
                in("x2") entry,
                in("x3") ctx,
                out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                options(nostack)
            );
        } else {
            core::arch::asm!(
                "smc #0",
                inout("x0") CPU_ON => ret,
                in("x1") target,
                in("x2") entry,
                in("x3") ctx,
                out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                options(nostack)
            );
        }
    }
    ret
}

/// Number of online cores.
pub fn nproc() -> usize {
    ONLINE.iter().filter(|c| c.load(Ordering::SeqCst)).count()
}

/// Run `n_chunks` chunks of `chunk_sz` iterations of `work` across all online
/// cores; returns (sum, cycles).
pub fn parallel_compute(n_chunks: usize, chunk_sz: usize) -> (u64, u64) {
    CHUNK.store(0, Ordering::SeqCst);
    DONE.store(0, Ordering::SeqCst);
    NCHUNKS.store(n_chunks, Ordering::SeqCst);
    CHUNK_SZ.store(chunk_sz, Ordering::SeqCst);
    for p in &PARTIAL {
        p.store(0, Ordering::SeqCst);
    }
    let t0 = cycles();
    GEN.fetch_add(1, Ordering::SeqCst); // publish a new job
    unsafe { core::arch::asm!("sev", options(nomem, nostack)) }; // wake idle cores
    process_chunks(0); // the primary works too
    while DONE.load(Ordering::Acquire) < n_chunks {
        core::hint::spin_loop();
    }
    let t1 = cycles();
    let sum = PARTIAL.iter().map(|p| p.load(Ordering::SeqCst)).fold(0u64, u64::wrapping_add);
    (sum, t1.wrapping_sub(t0))
}

/// Single-core baseline of the same workload (no fork).
pub fn single_compute(n_chunks: usize, chunk_sz: usize) -> (u64, u64) {
    let t0 = cycles();
    let mut sum = 0u64;
    for c in 0..n_chunks {
        sum = sum.wrapping_add(work(c * chunk_sz, (c + 1) * chunk_sz));
    }
    let t1 = cycles();
    (sum, t1.wrapping_sub(t0))
}

fn cycles() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// Boot self-test: bring-up count + a parallel speedup measurement.
pub fn selftest() {
    let n = nproc();
    crate::kprintln!("NPROC: {n}");
    // A CPU-bound workload split into chunks.
    let (s1, c1) = single_compute(64, 200_000);
    let (sp, cp) = parallel_compute(64, 200_000);
    let speedup = if cp > 0 { c1 * 100 / cp } else { 0 };
    crate::kprintln!(
        "SMP_BENCH: single={c1} cyc, parallel={cp} cyc on {n} cores -> {}.{:02}x (sums {})",
        speedup / 100, speedup % 100, if s1 == sp { "agree" } else { "DISAGREE" }
    );
    if n >= 2 && s1 == sp && cp < c1 {
        crate::kprintln!("SMP_OK: {n} cores online; parallel workload {}.{:02}x faster than 1 core", speedup / 100, speedup % 100);
    } else if n >= 2 {
        crate::kprintln!("SMP_PARTIAL: {n} cores online but no speedup (single={c1} parallel={cp})");
    } else {
        crate::kprintln!("SMP_FAIL: only {n} core online");
    }
}
