#![no_std]
#![cfg_attr(not(test), no_main)]
#![allow(dead_code)]

#[cfg(target_arch = "aarch64")]
mod boot;
mod hal;
mod memory;
mod process;
mod ipc;
mod scheduler;

#[cfg(target_arch = "aarch64")]
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

    // Spawn initial process (init)
    let init_pid = match process::spawn(
        "init",
        process::Priority::Normal,
        idle_loop,
        process::capability::CapabilitySet::empty(),
    ) {
        Some(pid) => {
            println!("[kernel] spawned init process: {:?}", pid);
            pid
        }
        None => {
            println!("[kernel] FATAL: could not spawn init process");
            loop {
                unsafe { core::arch::asm!("wfe") }
            }
        }
    };

    // Set as current and enqueue
    scheduler::set_current_pid(Some(init_pid));
    scheduler::enqueue(init_pid, process::Priority::Normal);

    // Main kernel loop (process switching happens in timer IRQ)
    loop {
        if scheduler::current_pid().is_none() {
            unsafe { core::arch::asm!("wfe") }
        }
    }
}

/// Idle loop process -- placeholder entry point
fn idle_loop() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[cfg(not(test))]
#[cfg(not(test))]
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

#[cfg(test)]
fn main() {}
