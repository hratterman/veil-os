#!/usr/bin/env bash
# Build user binaries (ELF -> flat .bin), then the kernel (which embeds
# hello.bin for the M9 check). Always build user first.
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

OBJCOPY="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"

(cd user && cargo build --release --quiet)
for bin in hello ls cat echo spin evil; do
    "$OBJCOPY" -O binary \
        "user/target/aarch64-unknown-none/release/$bin" \
        "user/target/aarch64-unknown-none/release/$bin.bin"
done

cargo build --quiet
