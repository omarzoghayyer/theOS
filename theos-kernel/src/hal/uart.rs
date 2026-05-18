use core::fmt;

const UART_DR: *mut u32 = 0x09000000 as *mut u32;

pub fn init() {}

pub fn write_byte(byte: u8) {
    unsafe {
        core::ptr::write_volatile(UART_DR, byte as u32);
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
