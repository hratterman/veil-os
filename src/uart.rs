//! Polled PL011 UART driver (TX only for now).
//!
//! QEMU virt guarantees UART0 at 0x0900_0000 (spec §4). Polled TX is all
//! M1 needs; RX and interrupts come later.

use core::fmt;
use core::ptr::{read_volatile, write_volatile};

/// QEMU virt PL011 UART0 base — one of the few addresses we may hard-code.
#[cfg(not(feature = "pi4"))]
pub const UART0_BASE: usize = 0x0900_0000;
/// BCM2711 PL011 UART0 (the documented SoC address; the Pi has no DTB at
/// the instant the first byte must go out).
#[cfg(feature = "pi4")]
pub const UART0_BASE: usize = 0xFE20_1000;

// Register offsets (PL011 TRM).
const DR: usize = 0x00; // data register
const FR: usize = 0x18; // flag register
const IBRD: usize = 0x24; // integer baud rate divisor
const FBRD: usize = 0x28; // fractional baud rate divisor
const LCR_H: usize = 0x2c; // line control
const CR: usize = 0x30; // control
const ICR: usize = 0x44; // interrupt clear

const FR_TXFF: u32 = 1 << 5; // TX FIFO full
const FR_BUSY: u32 = 1 << 3; // UART busy

const LCR_H_FEN: u32 = 1 << 4; // enable FIFOs
const LCR_H_WLEN_8: u32 = 0b11 << 5; // 8-bit words

const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

pub struct Pl011 {
    base: usize,
}

impl Pl011 {
    /// # Safety
    /// `base` must be the MMIO base of a PL011 and nothing else may drive it.
    pub const unsafe fn new(base: usize) -> Self {
        Self { base }
    }

    fn reg(&self, offset: usize) -> *mut u32 {
        (self.base + offset) as *mut u32
    }

    fn read(&self, offset: usize) -> u32 {
        unsafe { read_volatile(self.reg(offset)) }
    }

    fn write(&self, offset: usize, value: u32) {
        unsafe { write_volatile(self.reg(offset), value) }
    }

    /// 8N1, FIFOs on, TX+RX enabled. QEMU ignores the baud divisors but real
    /// hardware will not: 115200 baud from a 24 MHz clock on virt, from the
    /// 48 MHz UART clock on the Pi 4 (pinned by init_uart_clock in config.txt).
    pub fn init(&self) {
        #[cfg(feature = "pi4")]
        pi4_uart_gpio();
        self.write(CR, 0); // disable while configuring
        while self.read(FR) & FR_BUSY != 0 {}
        self.write(ICR, 0x7ff); // clear pending interrupts
        #[cfg(not(feature = "pi4"))]
        {
            self.write(IBRD, 13); // 24e6 / (16 * 115200)
            self.write(FBRD, 1);
        }
        #[cfg(feature = "pi4")]
        {
            self.write(IBRD, 26); // 48e6 / (16 * 115200)
            self.write(FBRD, 3);
        }
        self.write(LCR_H, LCR_H_FEN | LCR_H_WLEN_8);
        self.write(CR, CR_UARTEN | CR_TXE | CR_RXE);
    }

    pub fn put_byte(&self, byte: u8) {
        while self.read(FR) & FR_TXFF != 0 {}
        self.write(DR, byte as u32);
    }

    pub fn put_str(&self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.put_byte(b'\r');
            }
            self.put_byte(byte);
        }
    }
}

/// Route GPIO 14/15 to the PL011 (ALT0 TXD0/RXD0) with pulls off.
/// BCM2711 GPIO block; QEMU models these registers as plain RAM-like
/// stores, real hardware needs them before the first byte.
#[cfg(feature = "pi4")]
fn pi4_uart_gpio() {
    const GPIO_BASE: usize = 0xFE20_0000;
    const GPFSEL1: usize = GPIO_BASE + 0x04;
    const GPIO_PUP_PDN_CNTRL_REG0: usize = GPIO_BASE + 0xE4;
    unsafe {
        // FSEL14/FSEL15 (bits 12..18) = 0b100 (ALT0).
        let mut fsel = read_volatile(GPFSEL1 as *const u32);
        fsel = (fsel & !(0b111111 << 12)) | (0b100 << 12) | (0b100 << 15);
        write_volatile(GPFSEL1 as *mut u32, fsel);
        // Pull none on 14/15 (2 bits per pin: 28..32).
        let mut pull = read_volatile(GPIO_PUP_PDN_CNTRL_REG0 as *const u32);
        pull &= !(0b1111 << 28);
        write_volatile(GPIO_PUP_PDN_CNTRL_REG0 as *mut u32, pull);
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.put_str(s);
        Ok(())
    }
}

/// Console print macros. The PL011 driver is stateless over MMIO and TX is
/// polled, so constructing one per call site is safe — including from
/// exception handlers, which is the whole point.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut serial = unsafe { $crate::uart::Pl011::new($crate::uart::UART0_BASE) };
        let _ = write!(serial, $($arg)*);
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut serial = unsafe { $crate::uart::Pl011::new($crate::uart::UART0_BASE) };
        let _ = writeln!(serial, $($arg)*);
    }};
}
