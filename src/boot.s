// Veil boot code — entered by QEMU at _start (ELF entry, linked at 0x4010_0000).
//
// Contract on entry (QEMU virt, -kernel):
//   x0 = physical address of the DTB (QEMU places it at RAM base)
//   EL = 1 by default, 2 with -machine virtualization=on. Handle both.
//
// Responsibilities: park secondary cores, drop EL2 -> EL1 if needed,
// enable FP/SIMD at EL1, set up the boot stack, zero .bss, call kernel_main.

.section .text.boot, "ax"
.global _start

_start:
    // Preserve the DTB pointer across everything below (incl. eret).
    mov     x19, x0

    // Park all cores except core 0.
    mrs     x0, mpidr_el1
    and     x0, x0, #0xFF
    cbnz    x0, .Lpark

    // Which exception level are we at?
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.eq    .Lin_el2
    b.lo    .Lel1_entry         // already EL1
    // EL3 is not expected on virt with -kernel; nothing sane to do.
.Lpark:
    wfe
    b       .Lpark

.Lin_el2:
    // EL1 executes in AArch64.
    mov     x0, #(1 << 31)              // HCR_EL2.RW
    msr     hcr_el2, x0

    // Don't trap FP/SIMD or CP15 accesses to EL2.
    mov     x0, #0x33ff                 // CPTR_EL2 RES1 bits, no traps
    msr     cptr_el2, x0
    msr     hstr_el2, xzr

    // Let EL1 use the physical counter/timer; no virtual offset.
    mrs     x0, cnthctl_el2
    orr     x0, x0, #0b11               // EL1PCEN | EL1PCTEN
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr

    // Known-good SCTLR_EL1: MMU/caches off, mandatory RES1 bits set.
    ldr     x0, =0x30D00800
    msr     sctlr_el1, x0

    // Fake an exception return: EL1h, all interrupts masked.
    mov     x0, #0x3C5                  // DAIF masked | M[3:0] = EL1h
    msr     spsr_el2, x0
    adr     x0, .Lel1_entry
    msr     elr_el2, x0
    eret

.Lel1_entry:
    // Enable FP/SIMD at EL1 (rustc emits NEON for aarch64-unknown-none).
    mov     x0, #(3 << 20)              // CPACR_EL1.FPEN = 0b11
    msr     cpacr_el1, x0
    isb

    // Boot stack.
    ldr     x0, =__stack_top
    mov     sp, x0

    // Zero .bss (linker guarantees 16-byte-aligned bounds).
    ldr     x0, =__bss_start
    ldr     x1, =__bss_end
.Lbss_loop:
    cmp     x0, x1
    b.hs    .Lbss_done
    stp     xzr, xzr, [x0], #16
    b       .Lbss_loop
.Lbss_done:

    // kernel_main(dtb: *const u8) -> !
    mov     x0, x19
    bl      kernel_main
    b       .Lpark
