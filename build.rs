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
}
