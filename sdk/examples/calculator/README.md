# Veil Calculator — a third-party SDK app

A real calculator built with the [Veil SDK](../../README.md): a 4×4 button grid
feeds an expression string that a from-scratch recursive-descent parser
evaluates with correct `+ − × ÷` precedence and parentheses. The last result is
persisted with the SDK key/value store, so it survives reopening.

It is built and compiled **entirely outside Veil**, on a normal machine, then
installed into Veil with the package manager — proving the SDK is real.

## What it uses

- **Graphics:** `veil_sdk::clear`, `fill_rect`, `draw_text` (the button grid + display)
- **Input:** the exported `on_click(x, y)` event (button hit-testing)
- **Storage:** `veil_sdk::store_set` / `store_get` (persist the last result)
- **A real algorithm:** a recursive-descent expression evaluator (`expr → term → factor`)

## Build it (on your own machine)

```sh
# 1. Add the wasm target once
rustup target add wasm32-unknown-unknown

# 2. Compile the app to WebAssembly
rustup run 1.96.0 cargo build --release --target wasm32-unknown-unknown

# 3. Name the module + write a manifest
cp target/wasm32-unknown-unknown/release/veil_calc.wasm main.wasm
cat > manifest.toml <<'TOML'
name = "veil-calc"
version = "1.0.0"
description = "A calculator with a real expression parser, built with the Veil SDK"
author = "henry"
entry = "main.wasm"
permissions = ["storage"]
TOML

# 4. Package it as a .veil (a ZIP of manifest.toml + main.wasm)
python3 ../../../scripts/veil_pkg.py pack \
    --manifest manifest.toml --wasm main.wasm --out veil-calc.veil
```

## Install it in Veil

Serve the package from a registry and install it from inside a Veil session:

```sh
# on the host: serve a directory of .veil files
python3 scripts/veil_pkg.py serve --root ./registry --port 8080
```

```text
# inside Veil's shell:
pkg install veil-calc      # fetches <registry>/veil-calc.veil, installs to /apps/veil-calc
pkg list                   # veil-calc 1.0.0
```

It appears in the dock and runs in the on-OS WASM runtime — the button grid
renders through the SDK graphics calls and clicks evaluate the expression.

## Verified

Veil's boot self-test `SDK_APP_OK` parses this exact `.veil`, runs the compiled
WASM in the on-OS runtime, and checks the expression parser:
`2+3*4 = 14` (precedence), `(1+2)*4 = 12` (parentheses), `100-2*9 = 82`.
