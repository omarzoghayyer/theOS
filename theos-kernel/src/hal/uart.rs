// hal/uart.rs -- UART backend selection
//
// Two backends, selected at compile time:
//   - uart_exynos (default): real Samsung Exynos UART on Pixel 7 Pro /
//     Tensor G2 (gs201) hardware.
//   - uart_pl011 (`--features qemu`): ARM PL011 at 0x0900_0000, the UART
//     QEMU's `-M virt` machine actually provides. Needed because virt has
//     no Exynos UART at any address -- the Exynos driver's poll loop spins
//     forever against unmapped MMIO if run there.
//
// Both backends expose the same API (init, write_byte, write_str,
// UartWriter), so nothing above this module needs to know which is active.

#[cfg(not(feature = "qemu"))]
#[path = "uart_exynos.rs"]
mod uart_exynos;
#[cfg(not(feature = "qemu"))]
pub use uart_exynos::*;

#[cfg(feature = "qemu")]
#[path = "uart_pl011.rs"]
mod uart_pl011;
#[cfg(feature = "qemu")]
pub use uart_pl011::*;

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
