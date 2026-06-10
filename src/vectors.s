// AArch64 exception vector table for EL1 (installed into VBAR_EL1).
//
// Layout is architecturally fixed: 16 entries of 0x80 bytes — four groups
// (current EL w/ SP_EL0, current EL w/ SP_ELx, lower EL AArch64, lower EL
// AArch32) of four kinds (sync, IRQ, FIQ, SError). The kernel runs EL1h,
// so the live entries are the "current EL with SP_ELx" group; everything
// else lands in handle_invalid until M9 gives lower-EL entries a meaning.
//
// The trap frame saves ALL registers including the full SIMD/FP state:
// v8-v15 are only callee-saved in their low 64 bits, and an interrupt can
// land between any two instructions, so partial saves corrupt the
// interrupted code in ways that only show up much later. Layout must match
// `TrapFrame` in exceptions.rs exactly.

.equ TF_SIZE, 816

.macro kernel_entry
    sub     sp, sp, #TF_SIZE
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    str     x30, [sp, #240]
    mrs     x9, elr_el1
    mrs     x10, spsr_el1
    mrs     x11, esr_el1
    mrs     x12, far_el1
    mrs     x13, sp_el0
    stp     x9, x10, [sp, #248]
    stp     x11, x12, [sp, #264]
    str     x13, [sp, #280]
    stp     q0, q1, [sp, #288]
    stp     q2, q3, [sp, #320]
    stp     q4, q5, [sp, #352]
    stp     q6, q7, [sp, #384]
    stp     q8, q9, [sp, #416]
    stp     q10, q11, [sp, #448]
    stp     q12, q13, [sp, #480]
    stp     q14, q15, [sp, #512]
    stp     q16, q17, [sp, #544]
    stp     q18, q19, [sp, #576]
    stp     q20, q21, [sp, #608]
    stp     q22, q23, [sp, #640]
    stp     q24, q25, [sp, #672]
    stp     q26, q27, [sp, #704]
    stp     q28, q29, [sp, #736]
    stp     q30, q31, [sp, #768]
    mrs     x9, fpsr
    mrs     x10, fpcr
    // stp's imm7 range ends at 504; single str reaches these offsets.
    str     x9, [sp, #800]
    str     x10, [sp, #808]
.endm

// Restore order: sysregs first (the handler may have modified ELR, e.g. to
// skip a brk), then FP, then GPRs last since x9/x10 are scratch here.
.macro kernel_exit
    ldp     x9, x10, [sp, #248]
    msr     elr_el1, x9
    msr     spsr_el1, x10
    ldr     x9, [sp, #280]
    msr     sp_el0, x9
    ldr     x9, [sp, #800]
    ldr     x10, [sp, #808]
    msr     fpsr, x9
    msr     fpcr, x10
    ldp     q0, q1, [sp, #288]
    ldp     q2, q3, [sp, #320]
    ldp     q4, q5, [sp, #352]
    ldp     q6, q7, [sp, #384]
    ldp     q8, q9, [sp, #416]
    ldp     q10, q11, [sp, #448]
    ldp     q12, q13, [sp, #480]
    ldp     q14, q15, [sp, #512]
    ldp     q16, q17, [sp, #544]
    ldp     q18, q19, [sp, #576]
    ldp     q20, q21, [sp, #608]
    ldp     q22, q23, [sp, #640]
    ldp     q24, q25, [sp, #672]
    ldp     q26, q27, [sp, #704]
    ldp     q28, q29, [sp, #736]
    ldp     q30, q31, [sp, #768]
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    ldr     x30, [sp, #240]
    add     sp, sp, #TF_SIZE
    eret
.endm

// Invalid-entry stub: full save so the report shows real state, then a
// diverging Rust handler (prints frame + slot index, exits via semihosting).
.macro inv_stub idx
vec_invalid_\idx:
    kernel_entry
    mov     x0, sp
    mov     x1, #\idx
    bl      handle_invalid
    b       .
.endm

.section .text.vectors, "ax"

.balign 0x800
.global exception_vectors
exception_vectors:
    // Group 0: current EL, SP_EL0 — we never run on SP_EL0 in the kernel.
    b       vec_invalid_0
    .balign 0x80
    b       vec_invalid_1
    .balign 0x80
    b       vec_invalid_2
    .balign 0x80
    b       vec_invalid_3
    // Group 1: current EL, SP_ELx — the live kernel entries.
    .balign 0x80
    b       vec_el1_sync
    .balign 0x80
    b       vec_el1_irq
    .balign 0x80
    b       vec_invalid_6
    .balign 0x80
    b       vec_invalid_7
    // Group 2: lower EL, AArch64 — syscalls and user preemption (M9+).
    .balign 0x80
    b       vec_el0_sync
    .balign 0x80
    b       vec_el0_irq
    .balign 0x80
    b       vec_invalid_10
    .balign 0x80
    b       vec_invalid_11
    // Group 3: lower EL, AArch32 — never valid for this kernel.
    .balign 0x80
    b       vec_invalid_12
    .balign 0x80
    b       vec_invalid_13
    .balign 0x80
    b       vec_invalid_14
    .balign 0x80
    b       vec_invalid_15

vec_el1_sync:
    kernel_entry
    mov     x0, sp
    bl      handle_sync
    kernel_exit

vec_el1_irq:
    kernel_entry
    mov     x0, sp
    bl      handle_irq
    kernel_exit

vec_el0_sync:
    kernel_entry
    mov     x0, sp
    bl      handle_sync_el0
    kernel_exit

vec_el0_irq:
    kernel_entry
    mov     x0, sp
    bl      handle_irq
    kernel_exit

// Enter a user task: x0 = address of a fully-built TrapFrame sitting at
// (kernel stack top - frame size). Pops it and erets to EL0, leaving
// SP_EL1 at the stack top for this task's future traps.
.global user_eret
user_eret:
    mov     sp, x0
    kernel_exit

inv_stub 0
inv_stub 1
inv_stub 2
inv_stub 3
inv_stub 6
inv_stub 7
inv_stub 10
inv_stub 11
inv_stub 12
inv_stub 13
inv_stub 14
inv_stub 15
