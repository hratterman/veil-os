Read os-build-spec.md in full. That is your contract.

Begin with M1. Build the project scaffold: `.cargo/config.toml`, linker script, boot assembly (`_start` at `0x4010_0000`, stack setup, `.bss` zero, EL detect + drop to EL1, jump to Rust), a `panic_handler`, and a polled PL011 UART driver. Set up the Section 6 test harness (sentinel grep + semihosting exit).

Run it in QEMU with the M1 command from Section 7. Show me the exact command and the serial output proving `BOOT_OK`.

Do not proceed to M2 until M1 provably passes.
