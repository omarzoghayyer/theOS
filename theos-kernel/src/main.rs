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
    hal::uart::write_str("\n");
    hal::uart::write_str("  theOS kernel v0.1.0\n");
    hal::uart::write_str("  Target: Google Pixel 7 Pro (cheetah)\n");
    hal::uart::write_str("  Arch: aarch64 | Lang: Rust\n");
    hal::uart::write_str("\n");
    println!("[boot] DTB paddr: {:#018x}", _dtb_paddr);
    println!("[boot] initializing memory...");
    memory::init(_dtb_paddr);
    println!("[boot] memory: ok");
    println!("[boot] initializing process model...");
    process::init();
    println!("[boot] process: ok");
    println!("[boot] initializing IPC...");
    ipc::init();
    println!("[boot] ipc: ok");
    println!("[boot] initializing scheduler...");
    scheduler::init();
    println!("[boot] scheduler: ok");
    println!("[boot] theOS kernel ready");
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    hal::uart::write_str("\nKERNEL PANIC\n");
    if let Some(location) = info.location() {
        hal::uart::write_str(location.file());
        hal::uart::write_str("\n");
    }
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
