# Building apps for Veil OS

Veil runs **WebAssembly apps**. An app is a `wasm32` module that exports a few
functions and draws through the `veil_*` host ABI. You can write it in Rust or
C; this guide covers both. No prior Veil knowledge required.

## How a Veil app works

Your module exports:

| export | when it runs | required |
|--------|--------------|----------|
| `render()` | on open, and after every event — draw the whole UI here | **yes** |
| `init()` | once, when the app opens | no |
| `on_click(x, y)` | on a click at surface coords `(x, y)`, then `render()` runs | no |

App state lives in your module's **linear memory**, which Veil preserves between
frames — so a counter in a `static`/global just works.

You draw by calling host functions (declared in [`veil.h`](veil.h) for C, wrapped
by the [`veil-sdk`](veil-sdk) crate for Rust): `veil_clear`, `veil_fill_rect`,
`veil_draw_text`, plus `veil_log`, `veil_store_*` (persistent storage) and
`veil_http_get` / `veil_tcp_*` (network over the kernel's TCP/TLS stack).

## Rust

```sh
rustup target add wasm32-unknown-unknown
cd examples/hello-rust
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/hello_veil.wasm HELLO.WSM
```

The example ([`examples/hello-rust/src/lib.rs`](examples/hello-rust/src/lib.rs)):

```rust
#![no_std]
use veil_sdk as v;
static mut CLICKS: i32 = 0;

#[no_mangle] pub extern "C" fn render() {
    v::clear(v::color::BG);
    v::draw_text(20, 14, "Hello, Veil!", v::color::ACCENT, 28);
    v::fill_rect(20, 104, 150, 44, v::color::GREEN);
    v::draw_text(50, 115, "Click me", v::color::WHITE, 18);
    v::draw_int(96, 164, unsafe { CLICKS }, v::color::GOLD, 18);
}
#[no_mangle] pub extern "C" fn on_click(x: i32, y: i32) {
    if x >= 20 && x < 170 && y >= 104 && y < 148 { unsafe { CLICKS += 1 } }
}
```

## C

You need a wasm-capable clang (the [WASI SDK](https://github.com/WebAssembly/wasi-sdk)
or upstream LLVM — Apple's clang does not include the wasm backend):

```sh
cd examples/hello-c
clang --target=wasm32 -nostdlib -O2 \
  -Wl,--no-entry -Wl,--export=render -Wl,--export=on_click -Wl,--export=init \
  -Wl,--allow-undefined -I../.. -o hello.wasm hello.c
cp hello.wasm HELLOC.WSM
```

## Running it in Veil

1. Get the `.WSM` file onto Veil's disk — drop it on the upload page of a hosted
   session ([os.henryratterman.com](https://os.henryratterman.com)), or place it
   on the FAT16 image with `scripts/mkdisk.sh`.
2. Open the **Files** app, select your `.WSM`, press Enter (or click it).
3. The app opens in a window: `render()` draws the UI, and clicking dispatches
   `on_click`. `veil_log` output appears on the serial console.

## The ABI at a glance

See [`veil.h`](veil.h) for the full list. Highlights:

- **graphics**: `veil_width/height`, `veil_clear`, `veil_fill_rect`, `veil_draw_text`
- **storage**: `veil_store_set/get` (persists to the OS, per-app)
- **network**: `veil_http_get/post`, `veil_dns_resolve`, `veil_tcp_connect/send/recv/close`
- **log**: `veil_log`

## Limits (be kind to the interpreter)

Veil's WASM runtime is a from-scratch interpreter + AArch64 JIT. It supports the
MVP integer instruction set plus bulk-memory (`memory.copy`/`fill`). It does
**not** implement floats end-to-end, threads, SIMD, or the component model. Keep
apps `no_std` / `-nostdlib` and integer-only and they'll run great.
