// hal/uart_exynos.rs -- Samsung Exynos UART driver for Google Tensor G2 (gs201)
//
// Target: Google Pixel 7 Pro (cheetah), Tensor G2 (gs201)
// Default backend -- used unless built with `--features qemu` (see
// hal/uart.rs and uart_pl011.rs, which target QEMU's `-M virt` machine
// instead; that board has no Exynos UART at any address).
//
// Hardware facts VERIFIED against the gs201 device tree
// (arch/arm64/boot/dts/google/gs201.dtsi, node serial_0: uart@10A00000):
//
//   compatible            = "samsung,exynos-uart"
//   reg                   = <0x0 0x10A00000 0x100>   -> base 0x10A0_0000, 256-byte window
//   reg-io-width          = <4>                      -> 32-bit MMIO
//   samsung,fifo-size     = <256>                    -> TX/RX FIFO enabled
//   samsung,dbg-uart-ch                              -> this is the debug console channel (apc)
//   samsung,dbg-uart-baud = <115200>
//   samsung,dbg-word-len  = <8>                      -> 115200 8N1
//
// Register offsets are the standard Samsung Exynos UART layout, verified across
// QEMU exynos4210 model, Xen exynos4210-uart driver, and U-Boot serial_s5p:
//
//   ULCON   0x00  Line control
//   UCON    0x04  Control
//   UFCON   0x08  FIFO control
//   UMCON   0x0C  Modem control
//   UTRSTAT 0x10  TX/RX status (bit1 = TX buffer/shifter empty)
//   UERSTAT 0x14  Error status
//   UFSTAT  0x18  FIFO status (TX-full bit -- see note below)
//   UTXH    0x20  Transmit holding register
//   URXH    0x24  Receive holding register
//
// DESIGN NOTE -- why this driver does NOT initialize the UART:
//   The bootloader (ABL) leaves serial_0 running at 115200 8N1. That is exactly
//   how `fastboot oem uart` output reaches the host over this same line. So on
//   first bring-up we deliberately DO NOT touch ULCON/UCON/UFCON/clocks: we
//   inherit the bootloader's configuration and only push bytes out UTXH. This
//   avoids the gs201 clock tree (GATE_PERIC0_TOP1_USI0_UART), which we cannot
//   verify from public sources. Minimal driver = minimal ways to be wrong.
//
// UNVERIFIED BIT (flagged honestly): the exact TX-FIFO-full bit position in
//   UFSTAT for this Exynos variant. U-Boot's Exynos path uses (1 << 24). We use
//   that, but also provide a UTRSTAT-based fallback (see write_byte_trstat) in
//   case the FIFO-full bit differs on gs201. If output is garbled or the kernel
//   hangs on first print, swap write_byte -> write_byte_trstat as the first
//   diagnostic step.

use core::fmt;

/// Verified base address of serial_0 (apc debug UART) on gs201 / cheetah.
const UART_BASE: u64 = 0x10A0_0000;

// Register offsets (bytes) from UART_BASE. Standard Samsung Exynos layout.
const UTRSTAT: u64 = 0x10;
const UFSTAT:  u64 = 0x18;
const UTXH:    u64 = 0x20;

// UFSTAT: TX FIFO full. U-Boot Exynos path uses bit 24. UNVERIFIED for gs201.
const UFSTAT_TX_FIFO_FULL: u32 = 1 << 24;

// UTRSTAT: bit 1 = transmitter empty (buffer register empty). Used by the
// non-FIFO fallback path.
const UTRSTAT_TX_EMPTY: u32 = 1 << 1;

#[inline(always)]
fn reg(offset: u64) -> *mut u32 {
    (UART_BASE + offset) as *mut u32
}

/// No hardware init required -- bootloader already configured 115200 8N1.
/// Kept as a no-op so call sites in kernel_main don't change.
pub fn init() {}

/// Primary TX path: wait until the TX FIFO is not full, then write one byte.
pub fn write_byte(byte: u8) {
    unsafe {
        let ufstat = reg(UFSTAT) as *const u32;
        // Spin while TX FIFO is full.
        while core::ptr::read_volatile(ufstat) & UFSTAT_TX_FIFO_FULL != 0 {}
        core::ptr::write_volatile(reg(UTXH), byte as u32);
    }
}

/// Fallback TX path (non-FIFO semantics): wait until the transmitter is empty,
/// then write one byte. Use this if `write_byte` produces garbage or hangs --
/// it does not depend on the UFSTAT TX-full bit position.
#[allow(dead_code)]
pub fn write_byte_trstat(byte: u8) {
    unsafe {
        let utrstat = reg(UTRSTAT) as *const u32;
        while core::ptr::read_volatile(utrstat) & UTRSTAT_TX_EMPTY == 0 {}
        core::ptr::write_volatile(reg(UTXH), byte as u32);
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
