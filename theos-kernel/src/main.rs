#![no_std]
#![no_main]
#![allow(dead_code)]

mod boot;
mod hal;
mod memory;
mod process;
mod ipc;
mod scheduler;

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_dtb_paddr: u64) -> ! {
    hal::uart::init();
    hal::uart::write_str("theOS kernel v0.1.0\r\n");
    hal::uart::write_str("Target: Google Pixel 7 Pro\r\n");
    hal::uart::write_str("Arch: aarch64\r\n");
    hal::uart::write_str("[boot] theOS kernel ready\r\n");
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    hal::uart::write_str("KERNEL PANIC\r\n");
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
