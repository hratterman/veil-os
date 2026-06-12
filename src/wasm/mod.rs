//! M35 WebAssembly runtime — a from-scratch parser, stack-machine interpreter,
//! WASI host shims, and a single-pass AArch64 JIT. No crates. Runs WASI-style
//! modules (hello-world via fd_write) and JIT-compiles hot numeric functions
//! (Mandelbrot / Fibonacci) to native ARM64 for near-native speed.

pub mod host;
pub mod jit;
pub mod parser;
pub mod runtime;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Parse + run a module's `_start`/`main`, returning its captured stdout.
pub fn run(data: &[u8]) -> Result<String, String> {
    let module = parser::parse(data).ok_or("not a valid WASM module")?;
    let mut host = host::Host::new();
    let mut inst = runtime::Instance::new(&module, &mut host);
    let entry = inst
        .find_export("_start")
        .or_else(|| inst.find_export("main"))
        .ok_or("no _start/main export")?;
    inst.call(entry, &[], 0).ok_or("trapped during execution")?;
    Ok(inst.host.output.clone())
}

/// Call an exported function with args, interpreted. Returns its first result.
pub fn call_export(data: &[u8], name: &str, args: &[i64]) -> Option<i64> {
    let module = parser::parse(data)?;
    let mut host = host::Host::new();
    let mut inst = runtime::Instance::new(&module, &mut host);
    let idx = inst.find_export(name)?;
    inst.call(idx, args, 0)?.first().copied()
}

/// Try to JIT-compile an exported single-i32-arg → i32 function and run it
/// natively. Returns `(result, true)` if the JIT ran, or falls back to the
/// interpreter `(result, false)`.
pub fn call_export_jit(data: &[u8], name: &str, arg: i32) -> Option<(i64, bool)> {
    let module = parser::parse(data)?;
    let imp = module.num_imported_funcs;
    let idx = module.exports.iter().find(|e| e.kind == 0 && e.name == name)?.index;
    if idx >= imp {
        let f = &module.funcs[(idx - imp) as usize];
        if let Some(code) = jit::compile(f) {
            let r = code.run(arg);
            return Some((r as i64, true));
        }
    }
    call_export(data, name, &[arg as i64]).map(|r| (r, false))
}

/// One frame of a graphical WASM app (M41 step 12). The app's linear memory
/// holds its state across frames; `mem` carries it forward. If `cb` is given
/// (e.g. ("on_click", [x, y]) or ("init", [])) it runs before `render`.
pub struct AppFrame {
    pub px: Vec<u32>,
    pub mem: Vec<u8>,
    pub log: String,
}

pub fn app_has_render(data: &[u8]) -> bool {
    parser::parse(data).map(|m| m.exports.iter().any(|e| e.kind == 0 && e.name == "render")).unwrap_or(false)
}

/// Run one app frame: restore memory, run an optional callback, then `render`,
/// drawing into a fresh `w*h` host surface. Returns the surface + new memory.
pub fn app_frame(
    data: &[u8],
    w: usize,
    h: usize,
    mem: Option<Vec<u8>>,
    cb: Option<(&str, &[i64])>,
) -> Option<AppFrame> {
    let module = parser::parse(data)?;
    let mut host = host::Host::new_graphical(w, h);
    let mut inst = runtime::Instance::new(&module, &mut host);
    if let Some(m) = mem {
        let n = m.len().min(inst.mem.len());
        inst.mem[..n].copy_from_slice(&m[..n]);
    }
    if let Some((name, args)) = cb {
        if let Some(idx) = inst.find_export(name) {
            inst.call(idx, args, 0);
        }
    }
    if let Some(idx) = inst.find_export("render") {
        inst.call(idx, &[], 0);
    }
    let px = inst.host.fb.as_ref().map(|f| f.px.clone()).unwrap_or_default();
    Some(AppFrame { px, mem: inst.mem.clone(), log: inst.host.output.clone() })
}

/// A short human description of a module (for the WASM window header).
pub fn describe(data: &[u8]) -> String {
    match parser::parse(data) {
        Some(m) => {
            let exports: Vec<&str> = m.exports.iter().filter(|e| e.kind == 0).map(|e| e.name.as_str()).collect();
            alloc::format!(
                "wasm: {} funcs, {} imports, {} KiB mem, exports: {}",
                m.funcs.len(),
                m.num_imported_funcs,
                m.mem_min_pages * 64,
                exports.join(", ")
            )
        }
        None => "wasm: parse failed".to_string(),
    }
}
