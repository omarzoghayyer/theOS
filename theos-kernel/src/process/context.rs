// process/context.rs -- ARM64 Context Switch
//
// Saves and restores the CPU state of a kernel thread.
//
// ARM64 calling convention (AAPCS64):
//   Caller-saved:  x0-x18  (scratch)
//   Callee-saved:  x19-x28 (must save across switch)
//   Special:       x29=fp, x30=lr, sp
//
// Security: caller-saved registers are zeroed before restore to prevent
// register content leaking across context boundaries.

use core::arch::naked_asm;

/// Saved CPU context for one kernel thread.
/// Layout must match context_switch() assembly exactly -- do not reorder.
#[repr(C)]
pub struct CpuContext {
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub fp:  u64,  // x29
    pub lr:  u64,  // x30 -- return address; resuming thread continues here
    pub sp:  u64,
}

impl CpuContext {
    // Create a zeroed context (no authority, no state).
    pub const fn zeroed() -> Self {
        Self {
            x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0, lr: 0, sp: 0,
        }
    }

    // Prepare a context for a brand-new kernel thread.
    // On first switch-to: lr is restored -> ret branches to entry().
    // All other registers are zero -- no leakage from previous occupant.
    pub fn prepare(entry: fn() -> !, stack_top: u64) -> Self {
        Self {
            x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0,
            lr: entry as u64,
            sp: stack_top,
        }
    }
}

/// Switch CPU context from `current` to `next`.
///
/// Saves callee-saved registers into `current`, zeroes caller-saved registers
/// to prevent cross-context leakage, then restores `next` and returns into it.
///
/// # Safety
/// Both pointers must be valid, aligned, non-null.
/// Must be called from EL1 with interrupts disabled.
///
/// CpuContext offsets:
///   0x00 x19  0x08 x20  0x10 x21  0x18 x22
///   0x20 x23  0x28 x24  0x30 x25  0x38 x26
///   0x40 x27  0x48 x28  0x50 fp   0x58 lr
///   0x60 sp
#[unsafe(naked)]
pub unsafe extern "C" fn context_switch(
    current: *mut CpuContext,   // x0
    next:    *const CpuContext, // x1
) {
    naked_asm!(
        // Save current thread
        "stp x19, x20, [x0, #0x00]",
        "stp x21, x22, [x0, #0x10]",
        "stp x23, x24, [x0, #0x20]",
        "stp x25, x26, [x0, #0x30]",
        "stp x27, x28, [x0, #0x40]",
        "stp x29, x30, [x0, #0x50]",
        "mov x2, sp",
        "str x2,  [x0, #0x60]",

        // Zero caller-saved registers -- prevent cross-context leakage
        "mov x3,  xzr", "mov x4,  xzr", "mov x5,  xzr", "mov x6,  xzr",
        "mov x7,  xzr", "mov x8,  xzr", "mov x9,  xzr", "mov x10, xzr",
        "mov x11, xzr", "mov x12, xzr", "mov x13, xzr", "mov x14, xzr",
        "mov x15, xzr", "mov x16, xzr", "mov x17, xzr", "mov x18, xzr",

        // Restore next thread
        "ldp x19, x20, [x1, #0x00]",
        "ldp x21, x22, [x1, #0x10]",
        "ldp x23, x24, [x1, #0x20]",
        "ldp x25, x26, [x1, #0x30]",
        "ldp x27, x28, [x1, #0x40]",
        "ldp x29, x30, [x1, #0x50]",
        "ldr x2,  [x1, #0x60]",
        "mov sp,  x2",

        // Clear pointer registers -- don't leak addresses across boundary
        "mov x0, xzr",
        "mov x1, xzr",
        "mov x2, xzr",

        // Return into next thread (lr = next thread's saved return address)
        "ret",
    )
}
