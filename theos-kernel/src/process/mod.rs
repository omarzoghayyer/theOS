// process/mod.rs -- Capability-Based Process Model
//
// Every process in theOS has a set of capabilities.
// A capability is an unforgeable token granting access to a resource.
// No process has ambient authority -- cannot access anything
// it was not explicitly granted at creation time.
//
// This eliminates the privilege escalation attack class entirely:
// there is nothing to escalate to if you were never granted it.
//
// Phase 1 (current): stub
// Phase 2: CapabilitySet, ProcessDescriptor, context switch
// Phase 3: IPC capability passing between processes
// Phase 4: driver processes isolated by capability model

/// Initialize the process subsystem.
pub fn init() {
    crate::println!("[process] capability model: stub (Phase 1)");
    crate::println!("[process] context switch: Phase 2");
    crate::println!("[process] driver isolation: Phase 4");
}
