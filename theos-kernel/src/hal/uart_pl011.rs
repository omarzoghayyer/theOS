// hal/uart_pl011.rs -- ARM PL011 UART driver, QEMU `-M virt` machine only
//
// QEMU's aarch64 "virt" board wires a PL011 UART at 0x0900_0000 (see
// hw/arm/virt.c in QEMU: VIRT_UART). This is not Pixel 7 Pro / Tensor G2
// hardware -- real cheetah/gs201 uses a Samsung Exynos UART at a completely
// different address and register layout (see uart_exynos.rs). This driver
// exists so `cargo build --features qemu` + run_kernel.sh can validate
// kernel boot logic in QEMU without touching the real-hardware UART path.
//
// Selected at compile time via the `qemu` Cargo feature -- see hal/uart.rs.
//
// Register layout (PL011 TRM, offsets from UART_BASE):
//   UARTDR   0x000  Data register (write = TX, read = RX)
//   UARTFR   0x018  Flag register -- bit 5 (TXFF) = TX FIFO full

use core::fmt;

const UART_BASE: u64 = 0x0900_0000;

const UARTDR: u64 = 0x000;
const UARTFR: u64 = 0x018;

const UARTFR_TXFF: u32 = 1 << 5;

#[inline(always)]
fn reg(offset: u64) -> *mut u32 {
    (UART_BASE + offset) as *mut u32
}

/// QEMU's PL011 model comes up already enabled with a usable TX FIFO --
/// no baud/clock setup needed here.
pub fn init() {}

/// Wait until the TX FIFO is not full, then write one byte.
pub fn write_byte(byte: u8) {
    unsafe {
        let uartfr = reg(UARTFR) as *const u32;
        while core::ptr::read_volatile(uartfr) & UARTFR_TXFF != 0 {}
        core::ptr::write_volatile(reg(UARTDR), byte as u32);
    }
}

pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

pub struct UartWriter;

impl fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}
