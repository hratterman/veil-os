//! A WASM interpreter: linear memory, an operand stack, locals, structured
//! control flow (block/loop/if/br/br_if), function calls, i32/i64 arithmetic
//! and memory load/store. Imported functions are dispatched to the host (see
//! host.rs). Used directly, and as the correctness oracle / fallback for the JIT.

use super::host::Host;
use super::parser::Module;
use alloc::vec;
use alloc::vec::Vec;

const PAGE: usize = 65536;
const MAX_DEPTH: u32 = 256;

pub struct Instance<'a> {
    pub module: &'a Module,
    pub mem: Vec<u8>,
    pub globals: Vec<i64>,
    pub host: &'a mut Host,
    pub fuel: u64, // instruction budget, so a runaway module can't hang the OS
}

struct Ctrl {
    is_loop: bool,
    start: usize, // loop: re-entry pc; block/if: pc just after the matching end
    height: usize,
    arity: usize,
}

impl<'a> Instance<'a> {
    pub fn new(module: &'a Module, host: &'a mut Host) -> Instance<'a> {
        let mut mem = vec![0u8; module.mem_min_pages as usize * PAGE];
        if mem.is_empty() {
            mem = vec![0u8; PAGE]; // at least one page so loads don't panic
        }
        for seg in &module.data {
            let o = seg.offset as usize;
            if o + seg.bytes.len() <= mem.len() {
                mem[o..o + seg.bytes.len()].copy_from_slice(&seg.bytes);
            }
        }
        Instance { module, mem, globals: module.globals.clone(), host, fuel: 2_000_000_000 }
    }

    pub fn find_export(&self, name: &str) -> Option<u32> {
        self.module.exports.iter().find(|e| e.kind == 0 && e.name == name).map(|e| e.index)
    }

    /// Call a defined function by its global index (imports occupy 0..N).
    pub fn call(&mut self, func_idx: u32, args: &[i64], depth: u32) -> Option<Vec<i64>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let imp = self.module.num_imported_funcs;
        if func_idx < imp {
            // Imported function: hand off to the host.
            let field = &self.module.imports[func_idx as usize].field;
            let r = self.host.call(field, args, &mut self.mem);
            return Some(r.into_iter().collect());
        }
        let f = self.module.funcs.get((func_idx - imp) as usize)?.clone();
        let ftype = self.module.types.get(f.type_idx as usize)?.clone();
        let nparams = ftype.params.len();
        let nresults = ftype.results.len();

        // Locals = params then declared locals (zero-initialised).
        let mut locals: Vec<i64> = Vec::with_capacity(nparams + 4);
        locals.extend_from_slice(&args[..nparams.min(args.len())]);
        while locals.len() < nparams {
            locals.push(0);
        }
        for (count, _) in &f.locals {
            for _ in 0..*count {
                locals.push(0);
            }
        }

        let (else_of, end_of) = scan_blocks(&f.body);
        let mut stack: Vec<i64> = Vec::new();
        let mut ctrl: Vec<Ctrl> = Vec::new();
        let mut pc = 0usize;
        let body = &f.body;

        loop {
            self.fuel = self.fuel.checked_sub(1)?;
            let op = *body.get(pc)?;
            pc += 1;
            match op {
                0x00 => return None, // unreachable
                0x01 => {}           // nop
                0x02 | 0x03 | 0x04 => {
                    let bt = body[pc];
                    pc += 1;
                    let arity = if bt == 0x40 { 0 } else { 1 };
                    let opener = pc - 2;
                    if op == 0x04 {
                        // if: pop condition. `start` is just past the matching
                        // `end`; the else/end handler pops this frame.
                        let c = pop(&mut stack)?;
                        ctrl.push(Ctrl { is_loop: false, start: end_of[opener] + 1, height: stack.len(), arity });
                        if c == 0 {
                            // false: run the else-branch (its `end` pops us), or
                            // jump to the `end` itself when there's no else.
                            pc = match else_of[opener] {
                                Some(e) => e + 1,
                                None => end_of[opener],
                            };
                        }
                    } else {
                        let is_loop = op == 0x03;
                        let start = if is_loop { opener } else { end_of[opener] + 1 };
                        ctrl.push(Ctrl { is_loop, start, height: stack.len(), arity });
                    }
                }
                0x05 => {
                    // else: reached by falling out of the then-branch — exit the
                    // if (pop its frame) and skip past the matching end.
                    let c = ctrl.pop()?;
                    pc = c.start;
                }
                0x0b => {
                    // end of a block/loop/if (or the function).
                    if ctrl.pop().is_none() {
                        // function end: results are the top `nresults` values.
                        let n = stack.len();
                        return Some(stack.split_off(n.saturating_sub(nresults)));
                    }
                }
                0x0c => {
                    let l = uleb(body, &mut pc)? as usize;
                    do_branch(&mut stack, &mut ctrl, &mut pc, l)?;
                }
                0x0d => {
                    let l = uleb(body, &mut pc)? as usize;
                    let c = pop(&mut stack)?;
                    if c != 0 {
                        do_branch(&mut stack, &mut ctrl, &mut pc, l)?;
                    }
                }
                0x0f => {
                    let n = stack.len();
                    return Some(stack.split_off(n.saturating_sub(nresults)));
                }
                0x10 => {
                    let callee = uleb(body, &mut pc)? as u32;
                    let cty = self.callee_type(callee)?;
                    let np = cty.0;
                    let nr = cty.1;
                    let at = stack.len().checked_sub(np)?;
                    let cargs = stack.split_off(at);
                    let res = self.call(callee, &cargs, depth + 1)?;
                    for v in res.into_iter().take(nr) {
                        stack.push(v);
                    }
                }
                0x1a => {
                    pop(&mut stack)?;
                }
                0x1b => {
                    let c = pop(&mut stack)?;
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(if c != 0 { a } else { b });
                }
                0x20 => {
                    let i = uleb(body, &mut pc)? as usize;
                    stack.push(*locals.get(i)?);
                }
                0x21 => {
                    let i = uleb(body, &mut pc)? as usize;
                    let v = pop(&mut stack)?;
                    *locals.get_mut(i)? = v;
                }
                0x22 => {
                    let i = uleb(body, &mut pc)? as usize;
                    let v = *stack.last()?;
                    *locals.get_mut(i)? = v;
                }
                0x23 => {
                    let i = uleb(body, &mut pc)? as usize;
                    stack.push(*self.globals.get(i)?);
                }
                0x24 => {
                    let i = uleb(body, &mut pc)? as usize;
                    let v = pop(&mut stack)?;
                    *self.globals.get_mut(i)? = v;
                }
                // loads: align(uleb), offset(uleb)
                0x28..=0x35 => {
                    uleb(body, &mut pc)?;
                    let off = uleb(body, &mut pc)? as usize;
                    let addr = (pop(&mut stack)? as u32 as usize).wrapping_add(off);
                    let v = self.load(op, addr)?;
                    stack.push(v);
                }
                // stores: align, offset
                0x36..=0x3e => {
                    uleb(body, &mut pc)?;
                    let off = uleb(body, &mut pc)? as usize;
                    let val = pop(&mut stack)?;
                    let addr = (pop(&mut stack)? as u32 as usize).wrapping_add(off);
                    self.store(op, addr, val)?;
                }
                0x3f => {
                    pc += 1; // memory index 0x00
                    stack.push((self.mem.len() / PAGE) as i64);
                }
                0x40 => {
                    pc += 1;
                    let n = pop(&mut stack)? as u32 as usize;
                    let old = self.mem.len() / PAGE;
                    self.mem.resize(self.mem.len() + n * PAGE, 0);
                    stack.push(old as i64);
                }
                0x41 => {
                    let v = sleb(body, &mut pc)?;
                    stack.push(v as i32 as i64);
                }
                0x42 => {
                    let v = sleb(body, &mut pc)?;
                    stack.push(v);
                }
                _ => {
                    if !self.numeric(op, &mut stack)? {
                        return None; // unsupported opcode
                    }
                }
            }
        }
    }

    fn callee_type(&self, idx: u32) -> Option<(usize, usize)> {
        let imp = self.module.num_imported_funcs;
        let t = if idx < imp {
            self.module.imports[idx as usize].type_idx
        } else {
            *self.module.func_types.get((idx - imp) as usize)?
        };
        let ty = self.module.types.get(t as usize)?;
        Some((ty.params.len(), ty.results.len()))
    }

    fn load(&self, op: u8, addr: usize) -> Option<i64> {
        let m = &self.mem;
        Some(match op {
            0x28 => i32::from_le_bytes(m.get(addr..addr + 4)?.try_into().ok()?) as i64, // i32.load
            0x29 => i64::from_le_bytes(m.get(addr..addr + 8)?.try_into().ok()?), // i64.load
            0x2c => *m.get(addr)? as i8 as i64,  // i32.load8_s
            0x2d => *m.get(addr)? as i64,        // i32.load8_u
            0x2e => i16::from_le_bytes(m.get(addr..addr + 2)?.try_into().ok()?) as i64, // load16_s
            0x2f => u16::from_le_bytes(m.get(addr..addr + 2)?.try_into().ok()?) as i64, // load16_u
            _ => i32::from_le_bytes(m.get(addr..addr + 4)?.try_into().ok()?) as i64,
        })
    }

    fn store(&mut self, op: u8, addr: usize, val: i64) -> Option<()> {
        let m = &mut self.mem;
        match op {
            0x36 => m.get_mut(addr..addr + 4)?.copy_from_slice(&(val as i32).to_le_bytes()),
            0x37 => m.get_mut(addr..addr + 8)?.copy_from_slice(&val.to_le_bytes()),
            0x3a => *m.get_mut(addr)? = val as u8,
            0x3b => m.get_mut(addr..addr + 2)?.copy_from_slice(&(val as u16).to_le_bytes()),
            _ => m.get_mut(addr..addr + 4)?.copy_from_slice(&(val as i32).to_le_bytes()),
        }
        Some(())
    }

    /// i32/i64 numeric + comparison opcodes. Returns false for unknown ops.
    fn numeric(&mut self, op: u8, st: &mut Vec<i64>) -> Option<bool> {
        let a32 = |v: i64| v as i32;
        macro_rules! bin32 {
            ($f:expr) => {{
                let b = a32(pop(st)?);
                let a = a32(pop(st)?);
                st.push(($f(a, b)) as i32 as i64);
            }};
        }
        macro_rules! cmp32 {
            ($f:expr) => {{
                let b = a32(pop(st)?);
                let a = a32(pop(st)?);
                st.push(if $f(a, b) { 1 } else { 0 });
            }};
        }
        match op {
            0x45 => {
                let a = a32(pop(st)?);
                st.push(if a == 0 { 1 } else { 0 });
            }
            0x46 => cmp32!(|a, b| a == b),
            0x47 => cmp32!(|a, b| a != b),
            0x48 => cmp32!(|a: i32, b: i32| a < b),
            0x49 => cmp32!(|a: i32, b: i32| (a as u32) < (b as u32)),
            0x4a => cmp32!(|a: i32, b: i32| a > b),
            0x4b => cmp32!(|a: i32, b: i32| (a as u32) > (b as u32)),
            0x4c => cmp32!(|a: i32, b: i32| a <= b),
            0x4d => cmp32!(|a: i32, b: i32| (a as u32) <= (b as u32)),
            0x4e => cmp32!(|a: i32, b: i32| a >= b),
            0x4f => cmp32!(|a: i32, b: i32| (a as u32) >= (b as u32)),
            0x67 => {
                let a = a32(pop(st)?);
                st.push(a.leading_zeros() as i64);
            }
            0x68 => {
                let a = a32(pop(st)?);
                st.push(a.trailing_zeros() as i64);
            }
            0x69 => {
                let a = a32(pop(st)?);
                st.push(a.count_ones() as i64);
            }
            0x6a => bin32!(|a: i32, b: i32| a.wrapping_add(b)),
            0x6b => bin32!(|a: i32, b: i32| a.wrapping_sub(b)),
            0x6c => bin32!(|a: i32, b: i32| a.wrapping_mul(b)),
            0x6d => {
                let b = a32(pop(st)?);
                let a = a32(pop(st)?);
                if b == 0 {
                    return None;
                }
                st.push(a.wrapping_div(b) as i64);
            }
            0x6e => {
                let b = a32(pop(st)?) as u32;
                let a = a32(pop(st)?) as u32;
                if b == 0 {
                    return None;
                }
                st.push((a / b) as i32 as i64);
            }
            0x6f => {
                let b = a32(pop(st)?);
                let a = a32(pop(st)?);
                if b == 0 {
                    return None;
                }
                st.push(a.wrapping_rem(b) as i64);
            }
            0x70 => {
                let b = a32(pop(st)?) as u32;
                let a = a32(pop(st)?) as u32;
                if b == 0 {
                    return None;
                }
                st.push((a % b) as i32 as i64);
            }
            0x71 => bin32!(|a: i32, b: i32| a & b),
            0x72 => bin32!(|a: i32, b: i32| a | b),
            0x73 => bin32!(|a: i32, b: i32| a ^ b),
            0x74 => bin32!(|a: i32, b: i32| a.wrapping_shl(b as u32)),
            0x75 => bin32!(|a: i32, b: i32| a.wrapping_shr(b as u32)),
            0x76 => bin32!(|a: i32, b: i32| ((a as u32).wrapping_shr(b as u32)) as i32),
            0x77 => bin32!(|a: i32, b: i32| a.rotate_left(b as u32)),
            0x78 => bin32!(|a: i32, b: i32| a.rotate_right(b as u32)),
            // a handful of i64 ops (add/sub/mul/and/or/xor) on the full width.
            0x7c => {
                let b = pop(st)?;
                let a = pop(st)?;
                st.push(a.wrapping_add(b));
            }
            0x7d => {
                let b = pop(st)?;
                let a = pop(st)?;
                st.push(a.wrapping_sub(b));
            }
            0x7e => {
                let b = pop(st)?;
                let a = pop(st)?;
                st.push(a.wrapping_mul(b));
            }
            0xa7 => {
                let a = pop(st)?;
                st.push(a as i32 as i64);
            } // i32.wrap_i64
            0xac => {
                let a = a32(pop(st)?);
                st.push(a as i64);
            } // i64.extend_i32_s
            0xad => {
                let a = a32(pop(st)?) as u32;
                st.push(a as i64);
            } // i64.extend_i32_u
            _ => return Some(false),
        }
        Some(true)
    }
}

fn pop(st: &mut Vec<i64>) -> Option<i64> {
    st.pop()
}

fn do_branch(stack: &mut Vec<i64>, ctrl: &mut Vec<Ctrl>, pc: &mut usize, l: usize) -> Option<()> {
    // Drop l control frames, branch into the (l)-th from the top.
    for _ in 0..l {
        ctrl.pop()?;
    }
    let c = ctrl.last()?;
    let (height, arity, start, is_loop) = (c.height, c.arity, c.start, c.is_loop);
    // Keep `arity` result values; truncate to the label's stack height.
    let keep: Vec<i64> = stack.split_off(stack.len().saturating_sub(arity));
    stack.truncate(height);
    stack.extend(keep);
    if is_loop {
        // Re-enter the loop: jump to its body start (after the blocktype byte).
        *pc = start + 2;
    } else {
        ctrl.pop(); // exiting the block
        *pc = start;
    }
    Some(())
}

fn uleb(d: &[u8], pc: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let b = *d.get(*pc)?;
        *pc += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

fn sleb(d: &[u8], pc: &mut usize) -> Option<i64> {
    let mut result = 0i64;
    let mut shift = 0;
    let mut b;
    loop {
        b = *d.get(*pc)?;
        *pc += 1;
        result |= ((b & 0x7f) as i64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            break;
        }
    }
    if shift < 64 && (b & 0x40) != 0 {
        result |= -1i64 << shift;
    }
    Some(result)
}

/// Pre-scan a function body: matching `else`/`end` pc for each block opener.
fn scan_blocks(body: &[u8]) -> (Vec<Option<usize>>, Vec<usize>) {
    let mut else_of = vec![None; body.len()];
    let mut end_of = vec![0usize; body.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut pc = 0;
    while pc < body.len() {
        let op = body[pc];
        let opener = pc;
        let len = instr_len(body, pc);
        match op {
            0x02 | 0x03 | 0x04 => stack.push(opener),
            0x05 => {
                if let Some(&o) = stack.last() {
                    else_of[o] = Some(pc);
                }
            }
            0x0b => {
                if let Some(o) = stack.pop() {
                    end_of[o] = pc;
                }
            }
            _ => {}
        }
        pc += len;
    }
    (else_of, end_of)
}

/// Length in bytes of the instruction at `pc` (opcode + immediates).
fn instr_len(d: &[u8], pc: usize) -> usize {
    let op = d[pc];
    let mut p = pc + 1;
    let skip_uleb = |p: &mut usize| {
        while *p < d.len() && d[*p] & 0x80 != 0 {
            *p += 1;
        }
        if *p < d.len() {
            *p += 1;
        }
    };
    match op {
        0x02 | 0x03 | 0x04 => p += 1,          // blocktype byte
        0x0c | 0x0d | 0x10 => skip_uleb(&mut p),
        0x20..=0x24 => skip_uleb(&mut p),
        0x41 | 0x42 => skip_uleb(&mut p),      // const (sleb, same continuation bits)
        0x28..=0x3e => {
            skip_uleb(&mut p);
            skip_uleb(&mut p);
        }
        0x3f | 0x40 => p += 1,
        0x0e => {
            // br_table: count, count+1 ulebs
            let mut count = 0u64;
            let mut shift = 0;
            while p < d.len() {
                let b = d[p];
                p += 1;
                count |= ((b & 0x7f) as u64) << shift;
                if b & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            for _ in 0..count + 1 {
                skip_uleb(&mut p);
            }
        }
        _ => {}
    }
    p - pc
}
