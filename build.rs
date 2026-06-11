use std::path::Path;
use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let script = if std::env::var("CARGO_FEATURE_PI4").is_ok() {
        "linker-pi4.ld"
    } else {
        "linker.ld"
    };
    println!("cargo:rustc-link-arg=-T{dir}/{script}");
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=linker-pi4.ld");

    build_freetype(&dir);
}

/// Compile a minimal FreeType2 (TrueType + sfnt + psnames + smooth AA) from the
/// vendored C source into a static lib for the bare-metal AArch64 kernel, with
/// memory wired to the kernel heap (veil_* in src/freetype.rs).
fn build_freetype(dir: &str) {
    let out = std::env::var("OUT_DIR").unwrap();
    let ft = format!("{dir}/vendor/freetype");
    println!("cargo:rerun-if-changed={ft}/veil");
    println!("cargo:rerun-if-changed={ft}/cfg");

    let resource_dir = Command::new("clang")
        .arg("-print-resource-dir")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .expect("clang -print-resource-dir failed");

    let common: Vec<String> = vec![
        "-target".into(),
        "aarch64-unknown-none-elf".into(),
        "-ffreestanding".into(),
        "-fno-builtin".into(),
        "-nostdinc".into(),
        "-O2".into(),
        "-fno-stack-protector".into(),
        format!("-isystem{resource_dir}/include"),
        format!("-I{ft}/include"),
        format!("-I{ft}/cfg"),
        "-DFT2_BUILD_LIBRARY".into(),
        "-DFT_CONFIG_STANDARD_LIBRARY_H=<ftstdlib_custom.h>".into(),
        "-DFT_CONFIG_MODULES_H=<ftmodule_veil.h>".into(),
        "-DFT_CONFIG_OPTION_NO_ASSEMBLER".into(),
    ];

    let c_files = [
        "src/base/ftbase.c",
        "src/base/ftinit.c",
        "src/base/ftbitmap.c",
        "src/base/ftglyph.c",
        "src/base/ftdebug.c",
        "src/sfnt/sfnt.c",
        "src/truetype/truetype.c",
        "src/smooth/smooth.c",
        "src/psnames/psnames.c",
        "veil/veil_ftsystem.c",
        "veil/shim.c",
        "src/autofit/autofit.c",
        "veil/glyph.c",
    ];

    let mut objs = Vec::new();
    for f in c_files {
        let obj = format!("{out}/{}.o", f.replace('/', "_"));
        let status = Command::new("clang")
            .args(&common)
            .args(["-c", &format!("{ft}/{f}"), "-o", &obj])
            .status()
            .expect("clang failed to run");
        assert!(status.success(), "compiling {f} failed");
        objs.push(obj);
    }
    // setjmp/longjmp assembly.
    let sj = format!("{out}/setjmp.o");
    let status = Command::new("clang")
        .args(["-target", "aarch64-unknown-none-elf", "-c", &format!("{ft}/veil/setjmp.S"), "-o", &sj])
        .status()
        .expect("clang failed on setjmp.S");
    assert!(status.success(), "assembling setjmp.S failed");
    objs.push(sj);

    // Archive into libfreetype.a (prefer llvm-ar from the rust sysroot).
    let ar = find_ar();
    let lib = format!("{out}/libfreetype.a");
    let _ = std::fs::remove_file(&lib);
    let status = Command::new(&ar).args(["rcs", &lib]).args(&objs).status().expect("ar failed");
    assert!(status.success(), "archiving libfreetype.a failed");

    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=freetype");
}

fn find_ar() -> String {
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output().ok();
    let host = Command::new("rustc").arg("-vV").output().ok().and_then(|o| {
        String::from_utf8(o.stdout).ok().and_then(|s| {
            s.lines().find(|l| l.starts_with("host:")).map(|l| l[5..].trim().to_string())
        })
    });
    if let (Some(sr), Some(host)) = (sysroot, host) {
        if let Ok(s) = String::from_utf8(sr.stdout) {
            let p = format!("{}/lib/rustlib/{host}/bin/llvm-ar", s.trim());
            if Path::new(&p).exists() {
                return p;
            }
        }
    }
    "ar".into()
}
