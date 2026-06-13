//! Single-pass JavaScript -> AArch64 JIT for the numeric, call-free subset of
//! functions (the compute-heavy hot loops: arithmetic, comparisons, if/while/
//! for, local variables, return). Every value is an f64 held in an FP register
//! (d0..d7); params arrive through a pointer to an f64 array. Anything outside
//! the subset — calls, member/array/object access, closures, strings, &&/||,
//! bitwise, **  — makes `compile` bail (deopt) so the tree-walking interpreter
//! handles it. Code is written into a Vec<u32> in kernel RAM (EL1-executable,
//! no PXN), caches flushed, and called through a function pointer.
//!
//! This mirrors the WASM JIT (src/wasm/jit.rs) which hit ~2873x; for pure
//! numeric JS loops we see the same order-of-magnitude speedup over the
//! interpreter.

use super::ast::{Expr, Func, Pat, Stmt};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;

/// A compiled numeric function: `extern "C" fn(args: *const f64) -> f64`.
pub struct Code {
    buf: Vec<u32>,
    pub nparams: usize,
}

impl Code {
    /// Run with the given f64 arguments (missing args are read as whatever the
    /// caller's array holds; callers pass exactly `nparams`, zero-padded).
    pub fn run(&self, args: &[f64]) -> f64 {
        let f: extern "C" fn(*const f64) -> f64 = unsafe { core::mem::transmute(self.buf.as_ptr()) };
        f(args.as_ptr())
    }
}

// Up to 8 caller-saved FP registers (d0..d7) for params + locals + operand
// stack. Using only volatile registers avoids prologue save/restore. x9 is a
// scratch integer register for materialising f64 constants.
const MAX_D: u32 = 8;
const SCRATCH_X: u32 = 9;

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
    // ---- FP (double) ops ----
    fn fmov(&mut self, d: u32, n: u32) {
        if d != n {
            self.emit(0x1e604000 | (n << 5) | d); // FMOV Dd, Dn
        }
    }
    fn fmov_from_x(&mut self, d: u32, x: u32) {
        self.emit(0x9e670000 | (x << 5) | d); // FMOV Dd, Xn
    }
    fn fmov_zero(&mut self, d: u32) {
        self.emit(0x9e670000 | (31 << 5) | d); // FMOV Dd, XZR  -> 0.0
    }
    fn fadd(&mut self, d: u32, n: u32, m: u32) {
        self.emit(0x1e602800 | (m << 16) | (n << 5) | d);
    }
    fn fsub(&mut self, d: u32, n: u32, m: u32) {
        self.emit(0x1e603800 | (m << 16) | (n << 5) | d);
    }
    fn fmul(&mut self, d: u32, n: u32, m: u32) {
        self.emit(0x1e600800 | (m << 16) | (n << 5) | d);
    }
    fn fdiv(&mut self, d: u32, n: u32, m: u32) {
        self.emit(0x1e601800 | (m << 16) | (n << 5) | d);
    }
    fn fneg(&mut self, d: u32, n: u32) {
        self.emit(0x1e614000 | (n << 5) | d);
    }
    fn frintz(&mut self, d: u32, n: u32) {
        self.emit(0x1e65c000 | (n << 5) | d); // round toward zero
    }
    fn fcmp(&mut self, n: u32, m: u32) {
        self.emit(0x1e602000 | (m << 16) | (n << 5));
    }
    fn fcmp_zero(&mut self, n: u32) {
        self.emit(0x1e602008 | (n << 5));
    }
    fn scvtf(&mut self, d: u32, w: u32) {
        self.emit(0x1e620000 | (w << 5) | d); // SCVTF Dd, Wn (32-bit signed -> double)
    }
    fn cset(&mut self, w: u32, cond: u32) {
        self.emit(0x1a9f07e0 | ((cond ^ 1) << 12) | w); // CSINC Wd,WZR,WZR,!cond
    }
    // LDR Dt, [Xn, #imm*8]
    fn ldr_d(&mut self, t: u32, n: u32, imm_slots: u32) {
        self.emit(0xfd400000 | (imm_slots << 10) | (n << 5) | t);
    }
    // Materialise a 64-bit constant into Xd via MOVZ/MOVK.
    fn mov_x_imm64(&mut self, d: u32, v: u64) {
        let h0 = (v & 0xffff) as u32;
        let h1 = ((v >> 16) & 0xffff) as u32;
        let h2 = ((v >> 32) & 0xffff) as u32;
        let h3 = ((v >> 48) & 0xffff) as u32;
        self.emit(0xd2800000 | (h0 << 5) | d); // MOVZ Xd, #h0
        if h1 != 0 {
            self.emit(0xf2a00000 | (h1 << 5) | d); // MOVK Xd, #h1, LSL16
        }
        if h2 != 0 {
            self.emit(0xf2c00000 | (h2 << 5) | d); // MOVK Xd, #h2, LSL32
        }
        if h3 != 0 {
            self.emit(0xf2e00000 | (h3 << 5) | d); // MOVK Xd, #h3, LSL48
        }
    }
    fn load_f64(&mut self, d: u32, v: f64) {
        if v == 0.0 && v.is_sign_positive() {
            self.fmov_zero(d);
        } else {
            self.mov_x_imm64(SCRATCH_X, v.to_bits());
            self.fmov_from_x(d, SCRATCH_X);
        }
    }
    fn ret(&mut self) {
        self.emit(0xd65f03c0);
    }
    // B.cond placeholder (imm19), returns its position.
    fn bcond_placeholder(&mut self, cond: u32) -> usize {
        let p = self.pos();
        self.emit(0x54000000 | cond);
        p
    }
    fn b_placeholder(&mut self) -> usize {
        let p = self.pos();
        self.emit(0x14000000);
        p
    }
    fn b_to(&mut self, target: usize) {
        let off = (target as i64 - self.pos() as i64) as i32;
        self.emit(0x14000000 | (off as u32 & 0x03ff_ffff));
    }
    fn patch_b(&mut self, at: usize) {
        let off = (self.pos() as i64 - at as i64) as i32;
        self.code[at] = 0x14000000 | (off as u32 & 0x03ff_ffff);
    }
    fn patch_bcond(&mut self, at: usize) {
        let off = (self.pos() as i64 - at as i64) as i32;
        let base = self.code[at] & 0xff00001f; // keep opcode + cond
        self.code[at] = base | (((off as u32) & 0x7ffff) << 5);
    }
}

// AArch64 condition codes.
const EQ: u32 = 0;
const NE: u32 = 1;
const MI: u32 = 4; // <0 (signed) — unused
const GT: u32 = 12;
const GE: u32 = 10;
const LT: u32 = 11;
const LE: u32 = 13;

struct Ctx<'a> {
    a: Asm,
    locals: &'a BTreeMap<String, u32>,
    first_stack: u32,
    sp: u32,
    // loop control: (continue_target, break_patch_positions)
    loops: Vec<(usize, Vec<usize>)>,
    ok: bool,
}

impl<'a> Ctx<'a> {
    fn bail(&mut self) {
        self.ok = false;
    }
    fn top(&self) -> u32 {
        self.first_stack + self.sp
    }
    fn push_reg(&mut self) -> Option<u32> {
        let r = self.first_stack + self.sp;
        if r >= MAX_D {
            self.bail();
            return None;
        }
        self.sp += 1;
        Some(r)
    }
}

/// Compile a function if it fits the numeric subset; else None (deopt).
pub fn compile(f: &Func) -> Option<Code> {
    if f.is_generator || f.is_async {
        return None;
    }
    // Params must be plain identifiers.
    let mut locals: BTreeMap<String, u32> = BTreeMap::new();
    let mut next = 0u32;
    for p in &f.params {
        match p {
            Pat::Ident(n) => {
                if !locals.contains_key(n) {
                    locals.insert(n.clone(), next);
                    next += 1;
                }
            }
            _ => return None,
        }
    }
    let nparams = next;
    // Arrow with an expression body: synthesise `return <expr>`.
    let synth;
    let body: &[Stmt] = if let Some(eb) = &f.expr_body {
        synth = [Stmt::Return(Some((**eb).clone()))];
        &synth
    } else {
        &f.body
    };
    collect_locals(body, &mut locals, &mut next);
    if next > MAX_D {
        return None; // too many locals to fit in d0..d7
    }
    let mut ctx = Ctx {
        a: Asm { code: Vec::new() },
        locals: &locals,
        first_stack: next,
        sp: 0,
        loops: Vec::new(),
        ok: true,
    };
    emit_prologue(&mut ctx, nparams, next);
    emit_block(&mut ctx, body);
    if !ctx.ok {
        return None;
    }
    // Default return 0.0.
    ctx.a.fmov_zero(0);
    ctx.a.ret();
    finish(ctx.a, nparams as usize)
}

fn finish(a: Asm, nparams: usize) -> Option<Code> {
    let buf = a.code;
    if buf.is_empty() {
        return None;
    }
    unsafe { flush_icache(buf.as_ptr() as usize, buf.len() * 4) };
    Some(Code { buf, nparams })
}

fn emit_prologue(ctx: &mut Ctx, nparams: u32, nlocals: u32) {
    // params: d[i] = args[i]  (x0 = args pointer)
    for i in 0..nparams {
        ctx.a.ldr_d(i, 0, i);
    }
    // other locals: zero
    for i in nparams..nlocals {
        ctx.a.fmov_zero(i);
    }
}

fn collect_locals(stmts: &[Stmt], locals: &mut BTreeMap<String, u32>, next: &mut u32) {
    for s in stmts {
        collect_locals_stmt(s, locals, next);
    }
}

fn collect_locals_stmt(s: &Stmt, locals: &mut BTreeMap<String, u32>, next: &mut u32) {
    let mut add = |name: &str, locals: &mut BTreeMap<String, u32>, next: &mut u32| {
        if !locals.contains_key(name) {
            locals.insert(String::from(name), *next);
            *next += 1;
        }
    };
    match s {
        Stmt::Decl(ds) => {
            for (p, _) in ds {
                if let Pat::Ident(n) = p {
                    add(n, locals, next);
                }
            }
        }
        Stmt::If(_, t, e) => {
            collect_locals(t, locals, next);
            collect_locals(e, locals, next);
        }
        Stmt::While(_, b) | Stmt::Block(b) => collect_locals(b, locals, next),
        Stmt::For(init, _, _, b) => {
            if let Some(Stmt::Decl(ds)) = init.as_ref() {
                for (p, _) in ds {
                    if let Pat::Ident(n) = p {
                        add(n, locals, next);
                    }
                }
            }
            collect_locals(b, locals, next);
        }
        _ => {}
    }
}

fn emit_block(ctx: &mut Ctx, stmts: &[Stmt]) {
    for s in stmts {
        if !ctx.ok {
            return;
        }
        emit_stmt(ctx, s);
    }
}

fn emit_stmt(ctx: &mut Ctx, s: &Stmt) {
    match s {
        Stmt::Empty | Stmt::FuncDecl(_) => {}
        Stmt::Expr(e) => {
            if emit_expr(ctx, e).is_some() {
                ctx.sp = ctx.sp.saturating_sub(1); // discard value
            }
        }
        Stmt::Decl(ds) => {
            for (p, init) in ds {
                let Pat::Ident(name) = p else {
                    ctx.bail();
                    return;
                };
                let reg = match ctx.locals.get(name) {
                    Some(r) => *r,
                    None => {
                        ctx.bail();
                        return;
                    }
                };
                match init {
                    Some(e) => {
                        if let Some(r) = emit_expr(ctx, e) {
                            ctx.a.fmov(reg, r);
                            ctx.sp -= 1;
                        }
                    }
                    None => ctx.a.fmov_zero(reg),
                }
            }
        }
        Stmt::Return(e) => {
            match e {
                Some(e) => {
                    if let Some(r) = emit_expr(ctx, e) {
                        ctx.a.fmov(0, r);
                        ctx.sp -= 1;
                    }
                }
                None => ctx.a.fmov_zero(0),
            }
            ctx.a.ret();
        }
        Stmt::If(c, t, e) => {
            let Some(else_jmp) = emit_cond_jump_if_false(ctx, c) else { return };
            emit_block(ctx, t);
            if e.is_empty() {
                ctx.a.patch_bcond(else_jmp);
            } else {
                let end = ctx.a.b_placeholder();
                ctx.a.patch_bcond(else_jmp);
                emit_block(ctx, e);
                ctx.a.patch_b(end);
            }
        }
        Stmt::While(c, body) => {
            let start = ctx.a.pos();
            let Some(exit) = emit_cond_jump_if_false(ctx, c) else { return };
            ctx.loops.push((start, alloc::vec![]));
            emit_block(ctx, body);
            ctx.a.b_to(start);
            let (_, breaks) = ctx.loops.pop().unwrap();
            ctx.a.patch_bcond(exit);
            for b in breaks {
                ctx.a.patch_b(b);
            }
        }
        Stmt::For(init, cond, upd, body) => {
            if let Some(s) = init.as_ref() {
                emit_stmt(ctx, s);
            }
            let start = ctx.a.pos();
            let exit = match cond {
                Some(c) => emit_cond_jump_if_false(ctx, c),
                None => None,
            };
            if cond.is_some() && exit.is_none() {
                return;
            }
            ctx.loops.push((start, alloc::vec![]));
            emit_block(ctx, body);
            if let Some(u) = upd {
                if emit_expr(ctx, u).is_some() {
                    ctx.sp = ctx.sp.saturating_sub(1);
                }
            }
            ctx.a.b_to(start);
            let (_, breaks) = ctx.loops.pop().unwrap();
            if let Some(ex) = exit {
                ctx.a.patch_bcond(ex);
            }
            for b in breaks {
                ctx.a.patch_b(b);
            }
        }
        Stmt::Block(b) => emit_block(ctx, b),
        Stmt::Break(None) => {
            let at = ctx.a.b_placeholder();
            if let Some(l) = ctx.loops.last_mut() {
                l.1.push(at);
            } else {
                ctx.bail();
            }
        }
        Stmt::Continue(None) => {
            if let Some((target, _)) = ctx.loops.last() {
                let t = *target;
                ctx.a.b_to(t);
            } else {
                ctx.bail();
            }
        }
        // labeled break/continue, throw/try/for-of/for-in/switch: deopt
        _ => ctx.bail(),
    }
}

/// Emit `cond`, branch to a (to-be-patched) target when it is FALSE (== 0.0).
/// Returns the placeholder position, or None on bail.
fn emit_cond_jump_if_false(ctx: &mut Ctx, c: &Expr) -> Option<usize> {
    let r = emit_expr(ctx, c)?;
    ctx.a.fcmp_zero(r);
    ctx.sp -= 1;
    Some(ctx.a.bcond_placeholder(EQ)) // branch when equal to zero
}

/// Emit an expression, leaving its result in a fresh stack register. Returns
/// that register, or None if it bailed.
fn emit_expr(ctx: &mut Ctx, e: &Expr) -> Option<u32> {
    if !ctx.ok {
        return None;
    }
    match e {
        Expr::Num(n) => {
            let d = ctx.push_reg()?;
            ctx.a.load_f64(d, *n);
            Some(d)
        }
        Expr::Bool(b) => {
            let d = ctx.push_reg()?;
            ctx.a.load_f64(d, if *b { 1.0 } else { 0.0 });
            Some(d)
        }
        Expr::Ident(name) => {
            let src = *ctx.locals.get(name)?;
            let d = ctx.push_reg()?;
            ctx.a.fmov(d, src);
            Some(d)
        }
        Expr::Unary(op, inner) => {
            let r = emit_expr(ctx, inner)?;
            match *op {
                "-" => ctx.a.fneg(r, r),
                "+" => {}
                "!" => {
                    ctx.a.fcmp_zero(r);
                    ctx.a.cset(SCRATCH_X, EQ);
                    ctx.a.scvtf(r, SCRATCH_X);
                }
                _ => {
                    ctx.bail();
                    return None;
                }
            }
            Some(r)
        }
        Expr::Binary(op, a, b) => {
            let ra = emit_expr(ctx, a)?;
            let rb = emit_expr(ctx, b)?;
            let dst = ra;
            match *op {
                "+" => ctx.a.fadd(dst, ra, rb),
                "-" => ctx.a.fsub(dst, ra, rb),
                "*" => ctx.a.fmul(dst, ra, rb),
                "/" => ctx.a.fdiv(dst, ra, rb),
                "%" => {
                    // a - trunc(a/b)*b, using one temp reg above the operands
                    let tmp = ctx.first_stack + ctx.sp; // rb is top; tmp = rb+1
                    if tmp >= MAX_D {
                        ctx.bail();
                        return None;
                    }
                    ctx.a.fdiv(tmp, ra, rb);
                    ctx.a.frintz(tmp, tmp);
                    ctx.a.fmul(tmp, tmp, rb);
                    ctx.a.fsub(dst, ra, tmp);
                }
                "==" | "===" => cmp(ctx, ra, rb, EQ),
                "!=" | "!==" => cmp(ctx, ra, rb, NE),
                "<" => cmp(ctx, ra, rb, LT),
                ">" => cmp(ctx, ra, rb, GT),
                "<=" => cmp(ctx, ra, rb, LE),
                ">=" => cmp(ctx, ra, rb, GE),
                _ => {
                    ctx.bail();
                    return None;
                }
            }
            ctx.sp -= 1; // popped rb, result in ra
            Some(dst)
        }
        Expr::Logical(op, a, b) => {
            // Short-circuit && / || via branches, producing a numeric result.
            let ra = emit_expr(ctx, a)?;
            // keep ra as the result register; pop so b can reuse the slot
            ctx.sp -= 1;
            match *op {
                "&&" => {
                    ctx.a.fcmp_zero(ra);
                    let skip = ctx.a.bcond_placeholder(EQ); // a falsy -> keep a
                    let rb = emit_expr(ctx, b)?;
                    ctx.a.fmov(ra, rb);
                    ctx.sp -= 1;
                    ctx.a.patch_bcond(skip);
                }
                "||" => {
                    ctx.a.fcmp_zero(ra);
                    let skip = ctx.a.bcond_placeholder(NE); // a truthy -> keep a
                    let rb = emit_expr(ctx, b)?;
                    ctx.a.fmov(ra, rb);
                    ctx.sp -= 1;
                    ctx.a.patch_bcond(skip);
                }
                _ => {
                    ctx.bail();
                    return None;
                }
            }
            ctx.sp += 1; // result occupies ra
            Some(ra)
        }
        Expr::Cond(c, t, f) => {
            let rc = emit_expr(ctx, c)?;
            ctx.a.fcmp_zero(rc);
            ctx.sp -= 1;
            let else_jmp = ctx.a.bcond_placeholder(EQ);
            let rt = emit_expr(ctx, t)?;
            // move into a stable result reg = rc slot
            ctx.a.fmov(rc, rt);
            ctx.sp -= 1;
            let end = ctx.a.b_placeholder();
            ctx.a.patch_bcond(else_jmp);
            let rf = emit_expr(ctx, f)?;
            ctx.a.fmov(rc, rf);
            ctx.sp -= 1;
            ctx.a.patch_b(end);
            ctx.sp += 1;
            Some(rc)
        }
        Expr::Assign(op, target, value) => {
            let Expr::Ident(name) = &**target else {
                ctx.bail();
                return None;
            };
            let reg = *ctx.locals.get(name)?;
            if *op == "=" {
                let r = emit_expr(ctx, value)?;
                ctx.a.fmov(reg, r);
                Some(r)
            } else {
                // compound: reg = reg <op> value
                let cur = ctx.push_reg()?;
                ctx.a.fmov(cur, reg);
                let rv = emit_expr(ctx, value)?;
                let base = &op[..op.len() - 1];
                match base {
                    "+" => ctx.a.fadd(cur, cur, rv),
                    "-" => ctx.a.fsub(cur, cur, rv),
                    "*" => ctx.a.fmul(cur, cur, rv),
                    "/" => ctx.a.fdiv(cur, cur, rv),
                    _ => {
                        ctx.bail();
                        return None;
                    }
                }
                ctx.sp -= 1; // popped rv
                ctx.a.fmov(reg, cur);
                Some(cur)
            }
        }
        Expr::Update(op, prefix, target) => {
            let Expr::Ident(name) = &**target else {
                ctx.bail();
                return None;
            };
            let reg = *ctx.locals.get(name)?;
            let d = ctx.push_reg()?;
            ctx.a.fmov(d, reg); // old value
            let one = ctx.first_stack + ctx.sp;
            if one >= MAX_D {
                ctx.bail();
                return None;
            }
            ctx.a.load_f64(one, 1.0);
            let newv = ctx.first_stack + ctx.sp + 1;
            if newv >= MAX_D {
                ctx.bail();
                return None;
            }
            if *op == "++" {
                ctx.a.fadd(reg, reg, one);
            } else {
                ctx.a.fsub(reg, reg, one);
            }
            if *prefix {
                ctx.a.fmov(d, reg); // prefix returns new value
            }
            let _ = newv;
            Some(d)
        }
        _ => {
            ctx.bail();
            None
        }
    }
}

fn cmp(ctx: &mut Ctx, ra: u32, rb: u32, cond: u32) {
    ctx.a.fcmp(ra, rb);
    ctx.a.cset(SCRATCH_X, cond);
    ctx.a.scvtf(ra, SCRATCH_X);
}

// Suppress unused-const warning for MI (kept for documentation).
const _: u32 = MI;

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
