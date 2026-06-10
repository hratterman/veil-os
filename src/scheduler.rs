//! Tasks and scheduling (M9 cooperative, M11 preemptive).
//!
//! xv6-shaped: every task has its own kernel stack. A trap from EL0 lands
//! on that task's kernel stack (SP_EL1 stays parked there across eret);
//! switching tasks is `swtch` — save callee-saved regs + sp, load
//! another's — inside whatever handler is running. The trap frame never
//! moves: it lives at the top of each task's own kernel stack.
//!
//! Task 0 is the kernel itself (boot stack, EL1): the desktop loop. It is
//! always Ready or Running, so there is always somewhere to switch to.

use crate::exceptions::TrapFrame;
use crate::{frames, kprintln, paging};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const USER_BASE: usize = 0x20_0000_0000;
const USTACK_TOP: usize = 0x20_0100_0000;
const USTACK_PAGES: usize = 16; // 64 KiB user stack
const BSS_EXTRA_PAGES: usize = 16; // zeroed pages past the image for .bss
const KSTACK_FRAMES: usize = 16; // 64 KiB kernel stack per task
const TRAP_RESERVE: usize = 1024; // top-of-kstack area for the initial frame

/// Callee-saved register context for swtch: x19..x28, x29, x30(lr), sp.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    regs: [u64; 13],
}

core::arch::global_asm!(
    r#"
.global swtch
swtch:  // (x0: *mut Context to save into, x1: *const Context to load)
    stp x19, x20, [x0, #0]
    stp x21, x22, [x0, #16]
    stp x23, x24, [x0, #32]
    stp x25, x26, [x0, #48]
    stp x27, x28, [x0, #64]
    stp x29, x30, [x0, #80]
    mov x9, sp
    str x9, [x0, #96]
    ldp x19, x20, [x1, #0]
    ldp x21, x22, [x1, #16]
    ldp x23, x24, [x1, #32]
    ldp x25, x26, [x1, #48]
    ldp x27, x28, [x1, #64]
    ldp x29, x30, [x1, #80]
    ldr x9, [x1, #96]
    mov sp, x9
    ret
"#
);

unsafe extern "C" {
    fn swtch(save: *mut Context, load: *const Context);
    fn user_eret(frame: usize) -> !;
}

#[derive(PartialEq, Clone, Copy)]
pub enum State {
    Ready,
    Running,
    Zombie(i32),
}

pub struct File {
    pub data: Vec<u8>,
    pub pos: usize,
}

pub struct Task {
    pub pid: usize,
    pub name: String,
    pub state: State,
    ctx: Context,
    kstack: usize, // 0 for the kernel task (uses the boot stack)
    root: usize,   // TTBR0 for this task
    mapper: Option<paging::Mapper>,
    entry: usize,
    pub args: String,
    pub fds: Vec<Option<File>>,
}

// All access goes through critical sections (single core, IRQs masked).
static mut TASKS: Vec<Option<Task>> = Vec::new();
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static KERNEL_ROOT: AtomicUsize = AtomicUsize::new(0);
static PREEMPT: AtomicBool = AtomicBool::new(false);
static NEED_RESCHED: AtomicBool = AtomicBool::new(false);
static NEXT_PID: AtomicUsize = AtomicUsize::new(2);

fn critical<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        core::arch::asm!("msr daifset, #2", options(nomem, nostack));
    }
    let result = f();
    unsafe { core::arch::asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
    result
}

fn tasks() -> &'static mut Vec<Option<Task>> {
    unsafe { &mut *core::ptr::addr_of_mut!(TASKS) }
}

/// Register the boot context as task 0 (pid 1), the kernel task.
pub fn init(kernel_root: usize) {
    KERNEL_ROOT.store(kernel_root, Ordering::Relaxed);
    tasks().push(Some(Task {
        pid: 1,
        name: String::from("kernel"),
        state: State::Running,
        ctx: Context::default(),
        kstack: 0,
        root: kernel_root,
        mapper: None,
        entry: 0,
        args: String::new(),
        fds: Vec::new(),
    }));
}

pub fn current_pid() -> usize {
    critical(|| {
        tasks()[CURRENT.load(Ordering::Relaxed)]
            .as_ref()
            .map_or(0, |t| t.pid)
    })
}

pub fn with_current<R>(f: impl FnOnce(&mut Task) -> R) -> R {
    critical(|| f(tasks()[CURRENT.load(Ordering::Relaxed)].as_mut().unwrap()))
}

/// Load a flat user binary into a fresh address space; Ready to run.
pub fn spawn(bin: &[u8], name: &str, args: &str) -> Option<usize> {
    let kernel_root = KERNEL_ROOT.load(Ordering::Relaxed);
    let mut mapper = paging::Mapper::clone_kernel(kernel_root);

    let img_pages = bin.len().div_ceil(frames::FRAME_SIZE) + BSS_EXTRA_PAGES;
    for i in 0..img_pages {
        let frame = frames::alloc_zeroed()?;
        let off = i * frames::FRAME_SIZE;
        if off < bin.len() {
            let n = (bin.len() - off).min(frames::FRAME_SIZE);
            unsafe {
                core::ptr::copy_nonoverlapping(bin.as_ptr().add(off), frame as *mut u8, n)
            };
        }
        mapper.map_user_page(USER_BASE + off, frame, true);
    }
    for i in 0..USTACK_PAGES {
        let frame = frames::alloc_zeroed()?;
        mapper.map_user_page(USTACK_TOP - (i + 1) * frames::FRAME_SIZE, frame, false);
    }

    let kstack = frames::alloc_contiguous(KSTACK_FRAMES)?;
    unsafe {
        core::ptr::write_bytes(kstack as *mut u8, 0, KSTACK_FRAMES * frames::FRAME_SIZE)
    };
    let mut ctx = Context::default();
    ctx.regs[11] = task_trampoline as usize as u64; // lr
    // The trampoline builds the initial TrapFrame in the topmost bytes of
    // the kernel stack — its own SP must start BELOW that reserved area,
    // or writing the frame shreds the trampoline's live stack frame.
    ctx.regs[12] = (kstack + KSTACK_FRAMES * frames::FRAME_SIZE - TRAP_RESERVE) as u64; // sp

    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    let task = Task {
        pid,
        name: String::from(name),
        state: State::Ready,
        ctx,
        kstack,
        root: mapper.root(),
        mapper: Some(mapper),
        entry: USER_BASE,
        args: String::from(args),
        fds: Vec::new(),
    };
    critical(|| {
        let slots = tasks();
        kprintln!("SCHED: spawn pid {pid} '{name}' ({} bytes)", bin.len());
        if let Some(slot) = slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(task);
        } else {
            slots.push(Some(task));
        }
    });
    Some(pid)
}

/// A kernel-mode task (EL1, kernel address space): a fn pointer on its own
/// kernel stack, preemptively scheduled like everything else. Used by the
/// network services (M15's HTTP server runs as one of these).
pub fn spawn_kernel(name: &str, entry: fn()) -> Option<usize> {
    let kstack = frames::alloc_contiguous(KSTACK_FRAMES)?;
    unsafe {
        core::ptr::write_bytes(kstack as *mut u8, 0, KSTACK_FRAMES * frames::FRAME_SIZE)
    };
    let mut ctx = Context::default();
    ctx.regs[11] = kernel_task_trampoline as usize as u64; // lr
    ctx.regs[12] = (kstack + KSTACK_FRAMES * frames::FRAME_SIZE) as u64; // sp

    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    let task = Task {
        pid,
        name: String::from(name),
        state: State::Ready,
        ctx,
        kstack,
        root: KERNEL_ROOT.load(Ordering::Relaxed),
        mapper: None,
        entry: entry as usize,
        args: String::new(),
        fds: Vec::new(),
    };
    critical(|| {
        let slots = tasks();
        kprintln!("SCHED: spawn kernel task pid {pid} '{name}'");
        if let Some(slot) = slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(task);
        } else {
            slots.push(Some(task));
        }
    });
    Some(pid)
}

/// First code a kernel task runs (via swtch ret, IRQs masked): unmask and
/// call the entry fn at EL1 on this stack.
extern "C" fn kernel_task_trampoline() -> ! {
    let entry = critical(|| {
        tasks()[CURRENT.load(Ordering::Relaxed)].as_ref().unwrap().entry
    });
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack)) };
    let f: fn() = unsafe { core::mem::transmute(entry) };
    f();
    exit_current(0)
}

/// First code a new task runs (entered via swtch ret, IRQs masked, on its
/// own kernel stack): build an EL0 trap frame at the stack top and eret.
extern "C" fn task_trampoline() -> ! {
    let (entry, kstack_top) = critical(|| {
        let t = tasks()[CURRENT.load(Ordering::Relaxed)].as_ref().unwrap();
        (t.entry, t.kstack + KSTACK_FRAMES * frames::FRAME_SIZE)
    });
    let frame = (kstack_top - core::mem::size_of::<TrapFrame>()) as *mut TrapFrame;
    unsafe {
        core::ptr::write_bytes(frame as *mut u8, 0, core::mem::size_of::<TrapFrame>());
        (*frame).elr = entry as u64;
        (*frame).spsr = 0; // EL0t, all interrupts enabled
        (*frame).sp_el0 = USTACK_TOP as u64;
        user_eret(frame as usize)
    }
}

/// Pick the next Ready slot after `from`, round-robin. None = nobody else.
fn pick_next(from: usize) -> Option<usize> {
    let slots = tasks();
    let n = slots.len();
    for i in 1..=n {
        let idx = (from + i) % n;
        if let Some(t) = &slots[idx] {
            if t.state == State::Ready {
                return Some(idx);
            }
        }
    }
    None
}

fn switch_to(next: usize, save: *mut Context) {
    let slots = tasks();
    slots[next].as_mut().unwrap().state = State::Running;
    let root = slots[next].as_ref().unwrap().root;
    let load: *const Context = &slots[next].as_ref().unwrap().ctx;
    CURRENT.store(next, Ordering::Relaxed);
    paging::Mapper::switch_ttbr0(root);
    unsafe { swtch(save, load) };
}

/// Give up the CPU to the next Ready task (if any). Safe from task
/// context and from the tail of the IRQ handler.
pub fn yield_now() {
    critical(|| {
        let cur = CURRENT.load(Ordering::Relaxed);
        let Some(next) = pick_next(cur) else { return };
        let slots = tasks();
        let save: *mut Context = {
            let t = slots[cur].as_mut().unwrap();
            if t.state == State::Running {
                t.state = State::Ready;
            }
            &mut t.ctx
        };
        switch_to(next, save);
        // ...and eventually someone switches back to us, resuming here.
    })
}

/// Terminate the current task. Never returns; the scheduler forgets this
/// context (resources are freed later by reap()).
pub fn exit_current(code: i32) -> ! {
    unsafe { core::arch::asm!("msr daifset, #2", options(nomem, nostack)) };
    let cur = CURRENT.load(Ordering::Relaxed);
    {
        let t = tasks()[cur].as_mut().unwrap();
        kprintln!("SCHED: pid {} '{}' exited with code {code}", t.pid, t.name);
        t.state = State::Zombie(code);
    }
    let next = pick_next(cur).expect("no runnable task left (kernel task missing?)");
    let mut discard = Context::default();
    switch_to(next, &mut discard);
    unreachable!()
}

/// M9 driver: run user tasks (cooperatively) until none are Ready.
pub fn run_until_idle() {
    loop {
        let any = critical(|| {
            let cur = CURRENT.load(Ordering::Relaxed);
            pick_next(cur).is_some()
        });
        if !any {
            return;
        }
        yield_now();
    }
}

/// Reap one zombie: free its memory, return (pid, exit code).
pub fn reap() -> Option<(usize, i32)> {
    let task = critical(|| {
        let slots = tasks();
        for slot in slots.iter_mut() {
            if let Some(t) = slot {
                if let State::Zombie(_) = t.state {
                    return slot.take();
                }
            }
        }
        None
    })?;
    let State::Zombie(code) = task.state else { unreachable!() };
    if let Some(mapper) = task.mapper {
        mapper.free_user_space();
    }
    if task.kstack != 0 {
        frames::free(task.kstack, KSTACK_FRAMES);
    }
    Some((task.pid, code))
}

/// Anything alive besides the kernel task?
pub fn user_tasks_alive() -> bool {
    critical(|| {
        tasks().iter().flatten().any(|t| {
            t.pid != 1 && !matches!(t.state, State::Zombie(_))
        })
    })
}

// --- preemption (M11) -------------------------------------------------------

pub fn enable_preemption() {
    PREEMPT.store(true, Ordering::Relaxed);
}

/// Called from the timer tick (IRQ context).
pub fn tick() {
    if PREEMPT.load(Ordering::Relaxed) {
        NEED_RESCHED.store(true, Ordering::Relaxed);
    }
}

/// Called at the tail of handle_irq, after EOI.
pub fn maybe_preempt() {
    if NEED_RESCHED.swap(false, Ordering::Relaxed) {
        yield_now();
    }
}
