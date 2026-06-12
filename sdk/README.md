# Veil OS App SDK

Build apps for **[Veil OS](https://github.com/hratterman/veil-os)** — a graphical
operating system written from scratch (its own kernel, TCP/IP stack, browser,
and WebAssembly runtime). A Veil app is a `wasm32` module that draws through the
`veil_*` host ABI.

```
sdk/
├── veil.h                  # C bindings for the full ABI
├── veil-sdk/               # ergonomic Rust crate
├── GUIDE.md                # getting-started guide (Rust + C)
└── examples/
    ├── hello-rust/         # "Hello, Veil" in Rust  (the canonical example)
    └── hello-c/            # "Hello, Veil" in C
```

## Quick start (Rust)

```sh
rustup target add wasm32-unknown-unknown
cd examples/hello-rust
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/hello_veil.wasm HELLO.WSM
```

Drop `HELLO.WSM` onto a hosted Veil session
([os.henryratterman.com](https://os.henryratterman.com)) or onto the disk image,
then open it in the **Files** app. It draws a title, a button, and a click
counter — clicking the button increments the counter.

See **[GUIDE.md](GUIDE.md)** for the full walkthrough and the ABI reference.

## Fork this

This directory is a working template: copy `examples/hello-rust`, rename the
crate, and start drawing. The whole app is ~40 lines.

## License

MIT.
