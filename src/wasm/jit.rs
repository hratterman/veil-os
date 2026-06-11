//! Single-pass WASM -> AArch64 JIT. Compiles a `(i32) -> i32` function whose
//! body uses locals, i32 const/arithmetic/compare and structured control flow
//! (block/loop/if/else/br/br_if/return) into native ARM64, writes it into
//! executable heap memory (kernel RAM is EL1-executable — no PXN), flushes the
//! caches, and calls it through a function pointer. Anything outside that
//! subset (calls, memory ops, >9 live values) makes `compile` bail to the
//! interpreter. This is what makes a Mandelbrot/Fibonacci kernel run native-fast.

use super::parser::Func;
use alloc::vec::Vec;
use core::arch::asm;

pub struct Code {
    buf: Vec<u32>,
}

impl Code {
    pub fn run(&self, arg: i32) -> i32 {
        let f: extern "C" fn(i32) -> i32 = unsafe { core::mem::transmute(self.buf.as_ptr()) };
        f(arg)
    }
}

// First scratch register for our value model: x9. Locals occupy x9.., the
// operand stack continues above them. We use up to x9..x17 (9 registers).
const BASE: u32 = 9;
const MAX_REG: u32 = 17;

struct Asm {
    code: Vec<u32>,
}

impl Asm {
    fn emit(&mut self, w: u32) {
        self.code.push(w);
    }
    fn pos(&self) -> usize {
        self.code.len()
    }
    // MOV Wd, Wm  (ORR Wd, WZR, Wm)
    fn mov(&mut self, d: u32, m: u32) {
        if d != m {
            self.emit(0x2a0003e0 | (m << 16) | d);
        }
    }
    // MOV Wd, #imm32 via MOVZ/MOVK
    fn mov_imm(&mut self, d: u32, v: i32) {
        let u = v as u32;
        let lo = u & 0xffff;
        let hi = (u >> 16) & 0xffff;
        self.emit(0x52800000 | (lo << 5) | d); // MOVZ Wd, #lo
        if hi != 0 {
            self.emit(0x72a00000 | (hi << 5) | d); // MOVK Wd, #hi, LSL #16
        }
    }
    // ALU: op Wd, Wn, Wm
    fn alu(&mut self, base: u32, d: u32, n: u32, m: u32) {
        self.emit(base | (m << 16) | (n << 5) | d);
    }
    fn mul(&mut self, d: u32, n: u32, m: u32) {
        self.emit(0x1b007c00 | (m << 16) | (n << 5) | d); // MADD Wd,Wn,Wm,WZR
    }
    fn cmp(&mut self, n: u32, m: u32) {
        self.emit(0x6b00001f | (m << 16) | (n << 5)); // SUBS WZR, Wn, Wm
    }
    fn cset(&mut self, d: u32, cond: u32) {
        self.emit(0x1a9f07e0 | ((cond ^ 1) << 12) | d); // CSINC Wd,WZR,WZR,!cond
    }
    fn ret(&mut self) {
        self.emit(0xd65f03c0);
    }
    // placeholder branch; returns its position for later patching
    fn b_placeholder(&mut self) -> usize {
        let p = self.pos();
        self.emit(0x14000000);
        p
    }
    fn cbnz_placeholder(&mut self, t: u32) -> usize {
        let p = self.pos();
        self.emit(0x35000000 | t);
        p
    }
    fn cbz_placeholder(&mut self, t: u32) -> usize {
        let p = self.pos();
        self.emit(0x34000000 | t);
        p
    }
    // backward unconditional branch to `target`
    fn b_to(&mut self, target: usize) {
        let off = (target as i64 - self.pos() as i64) as i32;
        self.emit(0x14000000 | (off as u32 & 0x03ff_ffff));
    }
    fn cbnz_to(&mut self, t: u32, target: usize) {
        let off = (target as i64 - self.pos() as i64) as i32;
        self.emit(0x35000000 | (((off as u32) & 0x7ffff) << 5) | t);
    }
    fn patch_b(&mut self, at: usize) {
        let off = (self.pos() as i64 - at as i64) as i32;
        self.code[at] = 0x14000000 | (off as u32 & 0x03ff_ffff);
    }
    fn patch_cond(&mut self, at: usize) {
        // patch a CBZ/CBNZ placeholder (imm19) to branch to current pos
        let off = (self.pos() as i64 - at as i64) as i32;
        let base = self.code[at] & 0xff00001f; // keep opcode + Rt
        self.code[at] = base | (((off as u32) & 0x7ffff) << 5);
    }
}

struct Block {
    is_loop: bool,
    start: usize,          // loop body start (for backward branches)
    end_patches: Vec<usize>, // forward branches to the block's end
    else_patch: Option<usize>, // if: the CBZ to patch to else/end
}

/// Decode unsigned LEB128 at `*p`.
fn uleb(d: &[u8], p: &mut usize) -> Option<u64> {
    let mut r = 0u64;
    let mut s = 0;
    loop {
        let b = *d.get(*p)?;
        *p += 1;
        r |= ((b & 0x7f) as u64) << s;
        if b & 0x80 == 0 {
            return Some(r);
        }
        s += 7;
    }
}
fn sleb(d: &[u8], p: &mut usize) -> Option<i64> {
    let mut r = 0i64;
    let mut s = 0;
    let mut b;
    loop {
        b = *d.get(*p)?;
        *p += 1;
        r |= ((b & 0x7f) as i64) << s;
        s += 7;
        if b & 0x80 == 0 {
            break;
        }
    }
    if s < 64 && b & 0x40 != 0 {
        r |= -1i64 << s;
    }
    Some(r)
}

pub fn compile(f: &Func) -> Option<Code> {
    let nlocals_decl: u32 = f.locals.iter().map(|(c, _)| *c).sum();
    let nlocals = 1 + nlocals_decl; // one i32 param + declared locals
    let reg_local = |i: u32| BASE + i;
    let reg_stack = |sp: u32| BASE + nlocals + sp;
    if BASE + nlocals > MAX_REG {
        return None;
    }

    let mut a = Asm { code: Vec::new() };
    // Prologue: param n is in w0 -> local 0; zero the rest.
    a.mov(reg_local(0), 0);
    for i in 1..nlocals {
        a.mov_imm(reg_local(i), 0);
    }

    let mut blocks: Vec<Block> = Vec::new();
    let mut sp: u32 = 0; // compile-time operand-stack depth
    let body = &f.body;
    let mut p = 0usize;

    macro_rules! need {
        ($n:expr) => {
            if sp < $n {
                return None;
            }
        };
    }

    while p < body.len() {
        let op = body[p];
        p += 1;
        match op {
            0x01 => {} // nop
            0x02 => {
                p += 1; // blocktype
                blocks.push(Block { is_loop: false, start: 0, end_patches: Vec::new(), else_patch: None });
            }
            0x03 => {
                p += 1;
                blocks.push(Block { is_loop: true, start: a.pos(), end_patches: Vec::new(), else_patch: None });
            }
            0x04 => {
                p += 1;
                need!(1);
                sp -= 1;
                let patch = a.cbz_placeholder(reg_stack(sp));
                blocks.push(Block { is_loop: false, start: 0, end_patches: Vec::new(), else_patch: Some(patch) });
            }
            0x05 => {
                // else: jump past else-branch, patch the if's CBZ to here.
                let jmp = a.b_placeholder();
                let b = blocks.last_mut()?;
                b.end_patches.push(jmp);
                let ep = b.else_patch.take()?;
                a.patch_cond(ep);
            }
            0x0b => {
                // end of block/loop/if, or the function.
                match blocks.pop() {
                    Some(b) => {
                        if let Some(ep) = b.else_patch {
                            a.patch_cond(ep); // if with no else
                        }
                        for at in &b.end_patches {
                            a.patch_b(*at);
                        }
                    }
                    None => {
                        // function end: result (if any) in the top slot -> w0.
                        if sp > 0 {
                            a.mov(0, reg_stack(sp - 1));
                        }
                        a.ret();
                    }
                }
            }
            0x0c => {
                let l = uleb(body, &mut p)? as usize;
                let idx = blocks.len().checked_sub(1 + l)?;
                if blocks[idx].is_loop {
                    let t = blocks[idx].start;
                    a.b_to(t);
                } else {
                    let at = a.b_placeholder();
                    blocks[idx].end_patches.push(at);
                }
            }
            0x0d => {
                let l = uleb(body, &mut p)? as usize;
                need!(1);
                sp -= 1;
                let idx = blocks.len().checked_sub(1 + l)?;
                if blocks[idx].is_loop {
                    let t = blocks[idx].start;
                    a.cbnz_to(reg_stack(sp), t);
                } else {
                    let at = a.cbnz_placeholder(reg_stack(sp));
                    blocks[idx].end_patches.push(at);
                }
            }
            0x0f => {
                if sp > 0 {
                    a.mov(0, reg_stack(sp - 1));
                }
                a.ret();
            }
            0x20 => {
                let i = uleb(body, &mut p)? as u32;
                if i >= nlocals {
                    return None;
                }
                a.mov(reg_stack(sp), reg_local(i));
                sp += 1;
            }
            0x21 => {
                let i = uleb(body, &mut p)? as u32;
                need!(1);
                sp -= 1;
                if i >= nlocals {
                    return None;
                }
                a.mov(reg_local(i), reg_stack(sp));
            }
            0x22 => {
                let i = uleb(body, &mut p)? as u32;
                need!(1);
                if i >= nlocals {
                    return None;
                }
                a.mov(reg_local(i), reg_stack(sp - 1));
            }
            0x41 => {
                let v = sleb(body, &mut p)? as i32;
                if reg_stack(sp) > MAX_REG {
                    return None;
                }
                a.mov_imm(reg_stack(sp), v);
                sp += 1;
            }
            // binary i32 ALU
            0x6a | 0x6b | 0x6c | 0x71 | 0x72 | 0x73 | 0x74 | 0x75 | 0x76 => {
                need!(2);
                let d = reg_stack(sp - 2);
                let n = reg_stack(sp - 2);
                let m = reg_stack(sp - 1);
                match op {
                    0x6a => a.alu(0x0b000000, d, n, m), // add
                    0x6b => a.alu(0x4b000000, d, n, m), // sub
                    0x6c => a.mul(d, n, m),             // mul
                    0x71 => a.alu(0x0a000000, d, n, m), // and
                    0x72 => a.alu(0x2a000000, d, n, m), // orr
                    0x73 => a.alu(0x4a000000, d, n, m), // eor
                    0x74 => a.alu(0x1ac02000, d, n, m), // lslv
                    0x75 => a.alu(0x1ac02800, d, n, m), // asrv (shr_s)
                    0x76 => a.alu(0x1ac02400, d, n, m), // lsrv (shr_u)
                    _ => unreachable!(),
                }
                sp -= 1;
            }
            // comparisons -> 0/1
            0x46..=0x4f => {
                need!(2);
                let n = reg_stack(sp - 2);
                let m = reg_stack(sp - 1);
                a.cmp(n, m);
                let cond = match op {
                    0x46 => 0,  // eq
                    0x47 => 1,  // ne
                    0x48 => 11, // lt_s
                    0x49 => 3,  // lt_u (CC/LO)
                    0x4a => 12, // gt_s
                    0x4b => 8,  // gt_u (HI)
                    0x4c => 13, // le_s
                    0x4d => 9,  // le_u (LS)
                    0x4e => 10, // ge_s
                    0x4f => 2,  // ge_u (CS/HS)
                    _ => unreachable!(),
                };
                a.cset(reg_stack(sp - 2), cond);
                sp -= 1;
            }
            0x45 => {
                // i32.eqz
                need!(1);
                a.cmp(reg_stack(sp - 1), 31); // compare with WZR (x31)
                a.cset(reg_stack(sp - 1), 0); // eq
            }
            0x1a => {
                need!(1);
                sp -= 1;
            }
            _ => return None, // unsupported: bail to the interpreter
        }
    }
    // Fallthrough safety: if the body didn't end with a RET, add one.
    if a.code.last() != Some(&0xd65f03c0) {
        if sp > 0 {
            a.mov(0, reg_stack(sp - 1));
        }
        a.ret();
    }

    let buf = a.code;
    if buf.is_empty() {
        return None;
    }
    unsafe { flush_icache(buf.as_ptr() as usize, buf.len() * 4) };
    Some(Code { buf })
}

/// Make freshly-written instructions visible to the instruction stream:
/// clean D-cache to PoU, invalidate I-cache, then ISB.
unsafe fn flush_icache(start: usize, len: usize) {
    let end = start + len;
    let mut a = start & !15;
    while a < end {
        unsafe { asm!("dc cvau, {x}", x = in(reg) a) };
        a += 16;
    }
    unsafe { asm!("dsb ish") };
    let mut a = start & !15;
    while a < end {
        unsafe { asm!("ic ivau, {x}", x = in(reg) a) };
        a += 16;
    }
    unsafe {
        asm!("dsb ish");
        asm!("isb");
    }
}
