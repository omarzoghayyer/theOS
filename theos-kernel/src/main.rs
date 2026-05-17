// main.rs -- theOS Kernel Entry Point
//
// This is the first Rust code that runs after boot assembly.
// At this point:
//   - Stack is set up
//   - BSS is zeroed
//   - MMU is OFF (physical addresses only)
//   - Interrupts are OFF
//   - We are at EL1 (kernel privilege level)
//   - DTB address is in x0 (passed as dtb_paddr parameter)
//
// Our job in this file:
//   1. Initialize UART for debug output
//   2. Print boot banner
//   3. Hand off to kernel init sequence
//
// Security design:
//   - No_std: we do not link against any C runtime
//   - No heap allocator yet: stack only until memory::init()
//   - Panic handler: halts the CPU (no unwinding possible)
//
// Architecture: aarch64-unknown-none
//   "none" = no operating system (we ARE the OS)

#![no_std]          // No Rust standard library (no OS underneath us)
#![no_main]         // We define our own entry point (kernel_main)
#![allow(dead_code)]

// ── Module declarations ───────────────────────────────────────────────────

mod boot;
mod hal;
mod memory;
mod process;
mod ipc;
mod scheduler;

// ── Kernel entry point ────────────────────────────────────────────────────

/// Called from boot assembly (entry.s) after stack setup and BSS zeroing.
///
/// Parameters:
///   dtb_paddr: physical address of device tree blob
///              passed by bootloader in x0 register
///
/// This function must NEVER return. If it does, the CPU hangs in entry.s.
///
/// Safety: called from assembly with a valid stack. BSS is zeroed.
/// MMU is off -- all addresses are physical.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(dtb_paddr: u64) -> ! {
    // Step 1: Initialize UART for debug output
    // This is the very first thing -- if UART init fails,
    // we have no way to know anything went wrong.
    hal::uart::init();

    // Step 2: Print boot banner
    // If you see this on serial output, the kernel is alive.
    println!("                                              ");
    println!("  ████████╗██╗  ██╗███████╗ ██████╗ ███████╗");
    println!("     ██╔══╝██║  ██║██╔════╝██╔═══██╗██╔════╝");
    println!("     ██║   ███████║█████╗  ██║   ██║███████╗");
    println!("     ██║   ██╔══██║██╔══╝  ██║   ██║╚════██║");
    println!("     ██║   ██║  ██║███████╗╚██████╔╝███████║");
    println!("     ╚═╝   ╚═╝  ╚═╝╚══════╝ ╚═════╝ ╚══════╝");
    println!("                                              ");
    println!("  theOS Kernel v0.1.0 -- Booting...          ");
    println!("  Target: Google Pixel 7 Pro (cheetah)        ");
    println!("  Architecture: aarch64                       ");
    println!("  Language: Rust (memory safe by construction)");
    println!("                                              ");

    // Step 3: Log DTB address
    println!("[boot] DTB physical address: {:#018x}", dtb_paddr);

    // Step 4: Initialize memory subsystem
    println!("[boot] initializing memory manager...");
    memory::init(dtb_paddr);
    println!("[boot] memory: ok");

    // Step 5: Initialize process subsystem
    println!("[boot] initializing process model...");
    process::init();
    println!("[boot] process: ok");

    // Step 6: Initialize IPC
    println!("[boot] initializing IPC...");
    ipc::init();
    println!("[boot] ipc: ok");

    // Step 7: Initialize scheduler
    println!("[boot] initializing scheduler...");
    scheduler::init();
    println!("[boot] scheduler: ok");

    // Step 8: Boot complete
    println!("[boot] theOS kernel ready");
    println!("[boot] handing off to compositor...");

    // TODO: load theos-compositor as first userspace process
    // For now: halt in a low-power loop
    println!("[boot] HALT: compositor loader not yet implemented");

    loop {
        // WFE: wait for event -- low power idle
        // This is correct behavior for an idle kernel
        unsafe { core::arch::asm!("wfe") }
    }
}

// ── Panic handler ─────────────────────────────────────────────────────────

/// Called by Rust runtime when a panic occurs.
///
/// In a kernel, panics are unrecoverable -- we cannot unwind the stack
/// because there is no runtime to catch exceptions.
///
/// We print the panic location and halt forever.
/// On real hardware: this triggers a watchdog reset after timeout.
///
/// Security: we do NOT print panic messages in release builds
/// to avoid leaking kernel internals to an attacker who
/// triggered the panic intentionally.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // In debug builds: print panic info to UART
    #[cfg(debug_assertions)]
    {
        println!("");
        println!("╔══════════════════════════════════════════╗");
        println!("║         KERNEL PANIC -- HALTED           ║");
        println!("╚══════════════════════════════════════════╝");
        if let Some(location) = info.location() {
            println!("[panic] file: {}", location.file());
            println!("[panic] line: {}", location.line());
        }
        if let Some(msg) = info.message().as_str() {
            println!("[panic] message: {}", msg);
        }
    }

    // Halt forever -- watchdog will reset on real hardware
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
