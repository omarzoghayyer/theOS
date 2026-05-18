use core::fmt;

const UART_BASE: u64 = 0x09000000;
const UART_DR:   u64 = 0x000;
const UART_FR:   u64 = 0x018;
const UART_FR_TXFF: u32 = 1 << 5;

pub fn init() {}

pub fn write_byte(byte: u8) {
    unsafe {
        let fr = (UART_BASE + UART_FR) as *const u32;
        let dr = (UART_BASE + UART_DR) as *mut u32;
        while core::ptr::read_volatile(fr) & UART_FR_TXFF != 0 {}
        core::ptr::write_volatile(dr, byte as u32);
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

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut writer = $crate::hal::uart::UartWriter;
        core::write!(writer, $($arg)*).ok();
    }};
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::hal::uart::write_str("\n")
    };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut writer = $crate::hal::uart::UartWriter;
        core::write!(writer, $($arg)*).ok();
        $crate::hal::uart::write_str("\n");
    }};
}
