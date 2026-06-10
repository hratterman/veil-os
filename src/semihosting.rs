//! ARM semihosting — gives the test harness a real exit code from QEMU
//! (spec §6: self-reported exit). Requires `-semihosting` on the QEMU
//! command line; without it the HLT traps, so only call this under QEMU.

const SYS_EXIT: u64 = 0x18;
const ADP_STOPPED_APPLICATION_EXIT: u64 = 0x20026;

/// Terminate QEMU with `code` as its process exit status.
pub fn exit(code: u64) -> ! {
    // AArch64 SYS_EXIT takes a pointer to {reason, subcode}.
    let block: [u64; 2] = [ADP_STOPPED_APPLICATION_EXIT, code];
    unsafe {
        core::arch::asm!(
            "hlt #0xF000",
            in("x0") SYS_EXIT,
            in("x1") block.as_ptr() as u64,
            options(nostack)
        );
    }
    // Unreachable under QEMU with -semihosting; park otherwise.
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}
