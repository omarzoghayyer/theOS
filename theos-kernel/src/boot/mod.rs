// boot/mod.rs -- Boot sequence support
//
// Contains ARM64 boot assembly (entry.s) and boot-time helpers.
// After kernel_main() is called, this module's job is done.

// Include boot assembly -- this defines _start and kernel_entry
core::arch::global_asm!(include_str!("entry.s"));
