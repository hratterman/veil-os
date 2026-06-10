//! EL1 exception handling: trap frame, vector installation, Rust handlers.

use crate::{gic, kprintln, semihosting};

core::arch::global_asm!(include_str!("vectors.s"));

/// Saved CPU state on exception entry. Field offsets must match the
/// kernel_entry macro in vectors.s (frame size 816, 16-byte aligned).
#[repr(C)]
pub struct TrapFrame {
    pub x: [u64; 31],   // 0:   x0..x30
    pub elr: u64,       // 248: exception link register (resume PC)
    pub spsr: u64,      // 256: saved program status
    pub esr: u64,       // 264: exception syndrome
    pub far: u64,       // 272: fault address (valid for aborts only)
    pub sp_el0: u64,    // 280: user stack pointer
    pub q: [u128; 32],  // 288: full SIMD/FP register file
    pub fpsr: u64,      // 800
    pub fpcr: u64,      // 808
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 816);

unsafe extern "C" {
    static exception_vectors: u8;
}

/// Point VBAR_EL1 at our table.
pub fn install() {
    unsafe {
        let vbar = &exception_vectors as *const u8 as u64;
        core::arch::asm!(
            "msr vbar_el1, {0}",
            "isb",
            in(reg) vbar,
            options(nomem, nostack)
        );
    }
}

/// Unmask IRQs at EL1 (boot leaves all of DAIF masked).
pub fn enable_irqs() {
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack)) };
}

fn ec_name(ec: u64) -> &'static str {
    match ec {
        0x01 => "trapped WF*",
        0x0e => "illegal execution state",
        0x15 => "SVC (AArch64)",
        0x18 => "trapped MSR/MRS",
        0x20 => "instruction abort, lower EL",
        0x21 => "instruction abort, same EL",
        0x22 => "PC alignment fault",
        0x24 => "data abort, lower EL",
        0x25 => "data abort, same EL",
        0x26 => "SP alignment fault",
        0x3c => "BRK (AArch64)",
        _ => "unknown",
    }
}

#[unsafe(no_mangle)]
extern "C" fn handle_sync(tf: &mut TrapFrame) {
    let ec = (tf.esr >> 26) & 0x3f;
    let iss = tf.esr & 0x1ff_ffff;
    match ec {
        // SVC: ELR already points past the instruction; just report.
        0x15 => {
            kprintln!(
                "EXC: sync, EC={ec:#04x} ({}), imm={iss:#x}, ELR={:#x} -- handled",
                ec_name(ec),
                tf.elr
            );
        }
        // BRK: report and skip the instruction, else eret re-executes it.
        0x3c => {
            kprintln!(
                "EXC: sync, EC={ec:#04x} ({}), imm={iss:#x}, ELR={:#x} -- skipping",
                ec_name(ec),
                tf.elr
            );
            tf.elr += 4;
        }
        _ => {
            kprintln!(
                "FATAL EXC: sync, EC={ec:#04x} ({}), ISS={iss:#x}, ELR={:#x}, FAR={:#x}, SPSR={:#x}",
                ec_name(ec),
                tf.elr,
                tf.far,
                tf.spsr
            );
            semihosting::exit(1);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn handle_irq(_tf: &mut TrapFrame) {
    loop {
        let iar = gic::acknowledge();
        let intid = iar & 0x3ff;
        if intid == gic::SPURIOUS {
            break; // no more pending interrupts
        }
        if !gic::dispatch(intid) {
            kprintln!("IRQ: unexpected INTID {intid}");
        }
        gic::end_of_interrupt(iar);
    }
    // After EOI (so other tasks keep receiving ticks), honor a pending
    // reschedule. We switch away here; when switched back, kernel_exit
    // resumes whatever this IRQ interrupted.
    crate::scheduler::maybe_preempt();
}

/// Synchronous exceptions from EL0: syscalls, or a dying process.
#[unsafe(no_mangle)]
extern "C" fn handle_sync_el0(tf: &mut TrapFrame) {
    let ec = (tf.esr >> 26) & 0x3f;
    match ec {
        0x15 => crate::syscall::dispatch(tf), // SVC
        _ => {
            kprintln!(
                "USER FAULT: pid {} EC={ec:#04x} ({}), ISS={:#x}, ELR={:#x}, FAR={:#x}",
                crate::scheduler::current_pid(),
                ec_name(ec),
                tf.esr & 0x1ff_ffff,
                tf.elr,
                tf.far
            );
            crate::scheduler::exit_current(139); // SIGSEGV-ish
        }
    }
}

const VECTOR_NAMES: [&str; 16] = [
    "curr EL/SP0 sync", "curr EL/SP0 irq", "curr EL/SP0 fiq", "curr EL/SP0 serror",
    "curr EL/SPx sync", "curr EL/SPx irq", "curr EL/SPx fiq", "curr EL/SPx serror",
    "lower EL/a64 sync", "lower EL/a64 irq", "lower EL/a64 fiq", "lower EL/a64 serror",
    "lower EL/a32 sync", "lower EL/a32 irq", "lower EL/a32 fiq", "lower EL/a32 serror",
];

#[unsafe(no_mangle)]
extern "C" fn handle_invalid(tf: &mut TrapFrame, slot: u64) -> ! {
    kprintln!(
        "FATAL EXC: unhandled vector {slot} ({}), ESR={:#x}, ELR={:#x}, FAR={:#x}, SPSR={:#x}",
        VECTOR_NAMES[slot as usize & 15],
        tf.esr,
        tf.elr,
        tf.far,
        tf.spsr
    );
    semihosting::exit(1)
}
