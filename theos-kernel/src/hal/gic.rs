// hal/gic.rs -- ARM Generic Interrupt Controller v2 (GICv2)
//
// The GIC is the hardware that delivers interrupts to the CPU.
// Without it, the scheduler cannot preempt -- processes run forever.
//
// Problems this solves (from kernel issue tracker):
//
//   "Power management bolted on" [Linux, MEDIUM]
//     -> Timer interrupts drive the scheduler tick. The scheduler
//        can enforce quantum expiry and priority preemption in hardware.
//        A P0 audio process will always preempt a P2 daemon at quantum
//        boundary -- guaranteed by the interrupt controller, not by
//        cooperative yielding.
//
// GICv2 architecture:
//   Distributor (GICD) -- at gic_dist_base (0x08000000 on QEMU virt)
//     Controls which interrupts are enabled, their priority, and
//     which CPU they are routed to. One distributor per system.
//
//   CPU Interface (GICC) -- at gic_cpu_base (0x08010000 on QEMU virt)
//     Each CPU core has its own interface. Handles ACK, EOI, priority
//     masking. We use CPU0 interface (single-core for now).
//
// Interrupt IDs:
//   0-15:   SGI (Software Generated Interrupts) -- IPC between cores
//   16-31:  PPI (Private Peripheral Interrupts) -- per-core, incl. timer
//   32+:    SPI (Shared Peripheral Interrupts) -- devices
//
//   IRQ 27: ARM Generic Timer (PPI) -- our scheduler tick source
//           This is the architected timer, available on all ARMv8 cores.
//           On Pixel 7 Pro (Tensor G2 = Cortex-A55/A76): same IRQ.
//
// ARM Generic Timer:
//   Each core has a private countdown timer. We use the EL1 physical timer.
//   CNTPCT_EL0: current counter value (read-only)
//   CNTP_TVAL_EL0: timer value register (write to set countdown)
//   CNTP_CTL_EL0: control register (enable/unmask)
//   Fires IRQ 27 when CNTPCT reaches the programmed value.
//
// Phase 4: multi-core -- each core gets its own GIC CPU interface.

// -- GICv2 Distributor register offsets ---------------------------------------
const GICD_CTLR:      usize = 0x000; // Distributor Control
const GICD_TYPER:     usize = 0x004; // Interrupt Controller Type
const GICD_ISENABLER: usize = 0x100; // Interrupt Set-Enable (32 regs)
const GICD_ICENABLER: usize = 0x180; // Interrupt Clear-Enable
const GICD_IPRIORITYR: usize = 0x400; // Interrupt Priority (byte per IRQ)
const GICD_ITARGETSR:  usize = 0x800; // Interrupt Target (byte per IRQ)
const GICD_ICFGR:      usize = 0xC00; // Interrupt Configuration

// -- GICv2 CPU Interface register offsets -------------------------------------
const GICC_CTLR:  usize = 0x000; // CPU Interface Control
const GICC_PMR:   usize = 0x004; // Priority Mask Register
const GICC_IAR:   usize = 0x00C; // Interrupt Acknowledge
const GICC_EOIR:  usize = 0x010; // End of Interrupt

// -- ARM Generic Timer IRQ ----------------------------------------------------
const TIMER_IRQ: u32 = 27; // PPI -- EL1 physical timer

// -- GIC base addresses (set during init from DTB) ----------------------------
static mut GICD_BASE: u64 = 0;
static mut GICC_BASE: u64 = 0;

// -- Scheduler tick period ----------------------------------------------------
// How often the timer fires (in timer counter ticks).
// ARM generic timer frequency is typically 62.5MHz on QEMU virt.
// We read CNTFRQ_EL0 at runtime to get the actual frequency.
// Target: 1ms tick (1000 Hz) -- fine enough for 2ms P0 quantum.
const TICK_HZ: u64 = 1000; // 1ms ticks

static mut TIMER_FREQ: u64 = 62_500_000; // updated from CNTFRQ_EL0 at init

// -- MMIO helpers -------------------------------------------------------------

#[inline(always)]
unsafe fn gicd_write(offset: usize, val: u32) {
    let addr = (unsafe { GICD_BASE } + offset as u64) as *mut u32;
    unsafe { core::ptr::write_volatile(addr, val) };
}

#[inline(always)]
unsafe fn gicd_read(offset: usize) -> u32 {
    let addr = (unsafe { GICD_BASE } + offset as u64) as *const u32;
    unsafe { core::ptr::read_volatile(addr) }
}

#[inline(always)]
unsafe fn gicc_write(offset: usize, val: u32) {
    let addr = (unsafe { GICC_BASE } + offset as u64) as *mut u32;
    unsafe { core::ptr::write_volatile(addr, val) };
}

#[inline(always)]
unsafe fn gicc_read(offset: usize) -> u32 {
    let addr = (unsafe { GICC_BASE } + offset as u64) as *const u32;
    unsafe { core::ptr::read_volatile(addr) }
}

// -- GIC initialization -------------------------------------------------------

/// Initialize the GIC and ARM generic timer.
/// Must be called after DTB parsing (gic_dist_base and gic_cpu_base known).
/// Called from memory::init via the DTB info.
pub fn init(gic_dist_base: u64, gic_cpu_base: u64) {
    unsafe {
        GICD_BASE = gic_dist_base;
        GICC_BASE = gic_cpu_base;
    }

    crate::println!("[gic] GICD: {:#x}  GICC: {:#x}", gic_dist_base, gic_cpu_base);

    // Read GIC type -- tells us how many IRQs are supported
    let typer = unsafe { gicd_read(GICD_TYPER) };
    let it_lines = (typer & 0x1F) + 1;
    let num_irqs = it_lines * 32;
    crate::println!("[gic] {} IRQ lines supported", num_irqs);

    // --- Distributor init ---
    // Disable distributor before configuration
    unsafe { gicd_write(GICD_CTLR, 0) };

    // Set all interrupt priorities to 0xA0 (mid-priority, below mask)
    let mut i = 0;
    while i < num_irqs {
        let offset = GICD_IPRIORITYR + i as usize;
        unsafe { gicd_write(offset & !3, 0xA0A0A0A0) };
        i += 4;
    }

    // Route all SPIs to CPU0
    let mut i = 32u32;
    while i < num_irqs {
        let offset = GICD_ITARGETSR + i as usize;
        unsafe { gicd_write(offset & !3, 0x01010101) };
        i += 4;
    }

    // Disable all interrupts initially (clear-enable registers)
    let mut i = 0usize;
    while i < (num_irqs as usize + 31) / 32 {
        unsafe { gicd_write(GICD_ICENABLER + i * 4, 0xFFFFFFFF) };
        i += 1;
    }

    // Enable distributor
    unsafe { gicd_write(GICD_CTLR, 1) };

    // --- CPU Interface init ---
    // Set priority mask to 0xFF -- accept all priorities
    unsafe { gicc_write(GICC_PMR, 0xFF) };
    // Enable CPU interface
    unsafe { gicc_write(GICC_CTLR, 1) };

    crate::println!("[gic] distributor: enabled");
    crate::println!("[gic] cpu interface: enabled");

    // --- ARM Generic Timer init ---
    timer_init();
}

#[cfg(target_arch = "aarch64")]
fn timer_init() {
    // Read the timer frequency from hardware
    let freq: u64;
    unsafe {
        core::arch::asm!(
            "mrs {freq}, cntfrq_el0",
            freq = out(reg) freq,
        );
    }
    unsafe { TIMER_FREQ = freq; }

    let tick_period = freq / TICK_HZ;
    crate::println!("[gic] timer: {}Hz  tick period: {} cycles ({}Hz tick)",
        freq, tick_period, TICK_HZ);

    // Enable the timer IRQ (IRQ 27) in the distributor
    enable_irq(TIMER_IRQ);

    // Program the timer: set countdown value and enable
    unsafe {
        // Set the timer value (countdown in cycles)
        core::arch::asm!(
            "msr cntp_tval_el0, {val}",
            val = in(reg) tick_period,
        );
        // Enable timer, unmask interrupt: ENABLE=1, IMASK=0
        core::arch::asm!(
            "msr cntp_ctl_el0, {ctl}",
            ctl = in(reg) 1u64,
        );
    }

    crate::println!("[gic] timer IRQ {}: enabled", TIMER_IRQ);
}


#[cfg(not(target_arch = "aarch64"))]
fn timer_init() {
    // Stub for non-ARM64 (testing)
}

// -- IRQ management -----------------------------------------------------------

/// Enable a specific IRQ in the GIC distributor.
pub fn enable_irq(irq: u32) {
    let reg = (irq / 32) as usize;
    let bit = irq % 32;
    unsafe { gicd_write(GICD_ISENABLER + reg * 4, 1 << bit) };
}

/// Disable a specific IRQ.
pub fn disable_irq(irq: u32) {
    let reg = (irq / 32) as usize;
    let bit = irq % 32;
    unsafe { gicd_write(GICD_ICENABLER + reg * 4, 1 << bit) };
}

// -- IRQ handler --------------------------------------------------------------

/// Called from the ARM64 exception vector on every IRQ.
/// Acknowledges the interrupt, dispatches to the right handler,
/// signals End of Interrupt to the GIC.
///
/// Returns the IRQ number that was handled.
pub fn handle_irq() -> u32 {
    // Acknowledge -- reads the IRQ number and signals we're handling it
    let iar = unsafe { gicc_read(GICC_IAR) };
    let irq = iar & 0x3FF; // bits [9:0] = IRQ ID

    match irq {
        TIMER_IRQ => {
            #[cfg(target_arch = "aarch64")]
            handle_timer();
            #[cfg(not(target_arch = "aarch64"))]
            {
                if let Some(_next_pid) = crate::scheduler::tick(1000) {
                    crate::scheduler::set_current_pid(Some(_next_pid));
                }
            }
        }
        1022 | 1023 => {
            // Spurious interrupt -- ignore
        }
        _ => {
            crate::println!("[gic] unhandled IRQ {}", irq);
        }
    }

    // End of Interrupt -- allows GIC to deliver next interrupt
    unsafe { gicc_write(GICC_EOIR, iar) };

    irq
}

#[cfg(target_arch = "aarch64")]
fn handle_timer() {
    // Reprogram timer for next tick before doing any work
    // This keeps the tick rate accurate regardless of handler duration
    let tick_period = unsafe { TIMER_FREQ } / TICK_HZ;
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {val}",
            val = in(reg) tick_period,
        );
    }

    // Notify scheduler -- 1ms elapsed
    // If a context switch is needed, scheduler returns the next PID
    if let Some(next_pid) = crate::scheduler::tick(1000) {
        // Update current process in scheduler's atomic
        crate::scheduler::set_current_pid(Some(next_pid));
        // TODO Phase 2: Wire actual context switching here
        // This requires saving current process context and calling process::switch()
    }
}

// -- Exception vector ---------------------------------------------------------
// ARM64 requires a Vector Base Address Register (VBAR_EL1) pointing to
// an aligned table of exception handlers.
//
// Exception vector table layout (ARM64):
//   Each entry is 128 bytes (32 instructions).
//   16 entries: 4 sources × 4 types (Sync, IRQ, FIQ, SError)
//   Sources: Current EL with SP0, Current EL with SPx, Lower EL AArch64, Lower EL AArch32
//
// We only handle IRQ from Current EL with SPx (kernel IRQs).
// All other entries jump to unexpected_exception for now.

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    // Align to 2KB -- ARM64 VBAR_EL1 requirement
    ".balign 2048",
    ".global __exception_vector",
    "__exception_vector:",

    // --- Current EL, SP0 (4 entries × 128 bytes) ---
    // Sync
    ".balign 128",
    "b unexpected_exception",
    // IRQ
    ".balign 128",
    "b unexpected_exception",
    // FIQ
    ".balign 128",
    "b unexpected_exception",
    // SError
    ".balign 128",
    "b unexpected_exception",

    // --- Current EL, SPx (4 entries × 128 bytes) ---
    // Sync
    ".balign 128",
    "b unexpected_exception",
    // IRQ -- this is the one we handle
    ".balign 128",
    "b __irq_handler",
    // FIQ
    ".balign 128",
    "b unexpected_exception",
    // SError
    ".balign 128",
    "b unexpected_exception",

    // --- Lower EL, AArch64 (4 entries) ---
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",

    // --- Lower EL, AArch32 (4 entries) ---
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",
    ".balign 128",
    "b unexpected_exception",

    // --- IRQ handler stub ---
    // Save caller-saved registers, call Rust handler, restore, return
    "__irq_handler:",
    "sub sp, sp, #256",          // allocate exception frame
    "stp x0,  x1,  [sp, #0]",
    "stp x2,  x3,  [sp, #16]",
    "stp x4,  x5,  [sp, #32]",
    "stp x6,  x7,  [sp, #48]",
    "stp x8,  x9,  [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x30, [sp, #144]",  // x18, lr
    "bl  __rust_irq_handler",    // call Rust handler
    "ldp x0,  x1,  [sp, #0]",
    "ldp x2,  x3,  [sp, #16]",
    "ldp x4,  x5,  [sp, #32]",
    "ldp x6,  x7,  [sp, #48]",
    "ldp x8,  x9,  [sp, #64]",
    "ldp x10, x11, [sp, #80]",
    "ldp x12, x13, [sp, #96]",
    "ldp x14, x15, [sp, #112]",
    "ldp x16, x17, [sp, #128]",
    "ldp x18, x30, [sp, #144]",
    "add sp, sp, #256",
    "eret",                      // return from exception

    // --- Unexpected exception handler ---
    "unexpected_exception:",
    "b unexpected_exception",    // spin -- kernel panic in Phase 2
);

/// Rust-side IRQ handler -- called from __irq_handler assembly stub.
#[unsafe(no_mangle)]
pub extern "C" fn __rust_irq_handler() {
    handle_irq();
}

/// Install the exception vector and enable IRQs at EL1.
/// Must be called after GIC init.
#[cfg(target_arch = "aarch64")]
pub fn install_exception_vector() {
    unsafe {
        // Point VBAR_EL1 at our vector table
        core::arch::asm!(
            "adr {addr}, __exception_vector",
            "msr vbar_el1, {addr}",
            "isb",
            addr = out(reg) _,
        );
        // Unmask IRQs at EL1 (clear PSTATE.I bit)
        core::arch::asm!("msr daifclr, #2");
    }
    crate::println!("[gic] exception vector: installed");
    crate::println!("[gic] IRQs: unmasked at EL1");
}

#[cfg(not(target_arch = "aarch64"))]
pub fn install_exception_vector() {
    // Stub for non-ARM64 (testing)
}
