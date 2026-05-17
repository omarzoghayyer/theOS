// hal/uart.rs -- UART Serial Output
//
// Provides println!() for kernel debug output.
// This is the FIRST hardware we initialize -- if this works,
// we can debug everything else.
//
// Target: ARM PL011 UART (standard on ARM platforms)
// Pixel 7 Pro UART address: determined from DTB at runtime.
// For initial bring-up: using QEMU virt machine address (0x09000000)
// which lets us test without real hardware.
//
// Security assumptions (flag for audit):
//   - UART output is plaintext -- disable in production builds
//   - UART address is hardcoded for now -- must come from DTB in Phase 2
//   - No locking -- single-core boot only at this stage

use core::fmt;
use core::fmt::Write;

// PL011 UART register offsets (from base address)
const UART_DR:   u64 = 0x000; // Data Register (TX/RX)
const UART_FR:   u64 = 0x018; // Flag Register
const UART_IBRD: u64 = 0x024; // Integer Baud Rate Divisor
const UART_FBRD: u64 = 0x028; // Fractional Baud Rate Divisor
const UART_LCR:  u64 = 0x02C; // Line Control Register
const UART_CR:   u64 = 0x030; // Control Register

// Flag register bits
const UART_FR_TXFF: u32 = 1 << 5; // TX FIFO full
const UART_FR_BUSY: u32 = 1 << 3; // UART busy

// Control register bits
const UART_CR_UARTEN: u32 = 1 << 0; // UART enable
const UART_CR_TXE:    u32 = 1 << 8; // TX enable
const UART_CR_RXE:    u32 = 1 << 9; // RX enable

// Line control register bits
const UART_LCR_FEN:  u32 = 1 << 4; // FIFO enable
const UART_LCR_8BIT: u32 = 3 << 5; // 8-bit words

// UART base address:
// QEMU virt machine: 0x09000000 (for testing without hardware)
// Pixel 7 Pro:       from DTB -- updated in Phase 2
const UART_BASE: u64 = 0x09000000;

/// Read a UART register.
///
/// Safety: UART_BASE must be a valid MMIO address.
/// During boot (MMU off): physical address used directly.
unsafe fn read_reg(offset: u64) -> u32 {
    let addr = (UART_BASE + offset) as *const u32;
    unsafe { core::ptr::read_volatile(addr) }
}

/// Write a UART register.
///
/// Safety: UART_BASE must be a valid MMIO address.
unsafe fn write_reg(offset: u64, val: u32) {
    let addr = (UART_BASE + offset) as *mut u32;
    unsafe { core::ptr::write_volatile(addr, val) }
}

/// Initialize UART for 115200 baud, 8N1.
///
/// Must be called before any println!() output.
/// Called from kernel_main() before anything else.
pub fn init() {
    unsafe {
        // Disable UART before configuring
        write_reg(UART_CR, 0);

        // Wait for any in-progress transmission to finish
        while read_reg(UART_FR) & UART_FR_BUSY != 0 {}

        // Set baud rate: 115200 at 24MHz clock
        // Divisor = 24000000 / (16 * 115200) = 13.020833...
        // Integer part:    13
        // Fractional part: 0.020833 * 64 = 1.333... -> 1
        write_reg(UART_IBRD, 13);
        write_reg(UART_FBRD, 1);

        // Set line control: 8 bits, no parity, 1 stop bit, FIFO enabled
        write_reg(UART_LCR, UART_LCR_8BIT | UART_LCR_FEN);

        // Enable UART, TX, RX
        write_reg(UART_CR, UART_CR_UARTEN | UART_CR_TXE | UART_CR_RXE);
    }
}

/// Write a single byte to UART.
/// Blocks until TX FIFO has space.
pub fn write_byte(byte: u8) {
    unsafe {
        // Wait until TX FIFO is not full
        while read_reg(UART_FR) & UART_FR_TXFF != 0 {}
        // Write byte to data register
        write_reg(UART_DR, byte as u32);
    }
}

/// Write a string to UART.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r'); // UART convention: \n -> \r\n
        }
        write_byte(byte);
    }
}

/// UART writer -- implements fmt::Write so we can use write!() macro.
pub struct UartWriter;

impl fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

/// Global println! macro for kernel debug output.
/// Usage: println!("[boot] value: {}", x);
///
/// Security: disabled in release builds (no debug output in production)
#[macro_export]
macro_rules! println {
    () => {
        $crate::hal::uart::write_str("\n")
    };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut writer = $crate::hal::uart::UartWriter;
        core::writeln!(writer, $($arg)*).ok();
    }};
}
